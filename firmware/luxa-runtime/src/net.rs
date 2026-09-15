//! Wi-Fi association and the TCP/IP stack.
//!
//! Pure plumbing. Nothing in this file has an opinion about light.

use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_println::println;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{AuthenticationMethod, Config, Interface, WifiController};
use static_cell::StaticCell;

use crate::config::{HTTP_PORT, MAX_REQUEST_BYTES, WEBSOCKET_CLIENTS, WIFI_PASSWORD, WIFI_SSID};
use crate::{device, http};

/// Concurrent HTTP connections.
///
/// picoserve serves one connection per task, and a WebSocket client keeps its
/// task for as long as it is connected. Two more than the WebSocket clients
/// means a page can still load, and post while it loads, with every WebSocket
/// slot taken.
pub const WEB_TASKS: usize = WEBSOCKET_CLIENTS + 2;

/// Socket slots the network stack needs: one per web task, plus headroom for
/// DHCP and the sockets being torn down behind them.
const SOCKET_SLOTS: usize = WEB_TASKS + 3;

/// Brings up the station interface and the IP stack.
///
/// Returns the stack plus the two long-lived drivers the caller must spawn.
pub fn init(
    wifi: WIFI<'static>,
    seed: u64,
) -> (
    Stack<'static>,
    Runner<'static, Interface>,
    WifiController<'static>,
) {
    let mut controller =
        WifiController::new(wifi, Default::default()).expect("wifi controller init failed");

    let station = StationConfig::default()
        .with_ssid(WIFI_SSID)
        // Wokwi's gateway is open. On a real network this becomes WPA2 and
        // the credentials stop being compile-time constants.
        .with_auth_method(if WIFI_PASSWORD.is_empty() {
            AuthenticationMethod::None
        } else {
            AuthenticationMethod::Wpa2Personal
        })
        .with_password(WIFI_PASSWORD.into());

    controller
        .set_config(&Config::Station(station))
        .expect("wifi station config rejected");

    let interface = Interface::station();
    device::set_mac(interface.mac_address());

    static RESOURCES: StaticCell<StackResources<SOCKET_SLOTS>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        interface,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    (stack, runner, controller)
}

/// Keeps the station associated, reconnecting if the link drops.
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(_) => {
                println!("wifi: associated with {WIFI_SSID}");
                // Park until the link goes away, then fall through and retry.
                let _ = controller.wait_for_disconnect_async().await;
                println!("wifi: link lost, reconnecting");
            }
            Err(err) => {
                println!("wifi: connect failed ({err:?}), retrying");
                Timer::after(Duration::from_secs(3)).await;
            }
        }
    }
}

/// Drives the embassy-net stack.
#[embassy_executor::task]
pub async fn net(mut runner: Runner<'static, Interface>) -> ! {
    runner.run().await
}

/// Waits for DHCP and announces the URL to open.
pub async fn wait_for_address(stack: Stack<'static>) {
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        let address = config.address.address();
        device::set_address(address.octets());
        println!("luxa: open http://{address}/");
    }
}

/// Serves HTTP on one connection at a time; several of these run in parallel.
#[embassy_executor::task(pool_size = WEB_TASKS)]
pub async fn web(id: usize, stack: Stack<'static>) -> ! {
    let app = http::router();

    // Close after each response: with only a couple of tasks, a keep-alive
    // connection from one browser tab would starve everything else.
    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Duration::from_secs(10),
        persistent_start_read_request: Duration::from_secs(2),
        read_request: Duration::from_secs(5),
        write: Duration::from_secs(5),
    })
    .close_connection_after_response();

    let mut rx = [0u8; 1024];
    let mut tx = [0u8; 1024];
    // The request head, followed by a body of up to `MAX_REQUEST_BYTES`,
    // which the JSON API parses in place.
    let mut http_buffer = [0u8; MAX_REQUEST_BYTES + 1024];

    loop {
        picoserve::Server::new(&app, &config, &mut http_buffer)
            .listen_and_serve(id, stack, HTTP_PORT, &mut rx, &mut tx)
            .await;
    }
}
