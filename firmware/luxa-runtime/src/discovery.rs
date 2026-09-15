//! Discovery: advertising the fixture over mDNS, so clients find it without
//! being told an address.
//!
//! The fixture answers as `luxa-xxxxxx.local`, from the tail of its MAC
//! address, and advertises two services on the HTTP port: `_http._tcp` for
//! browsers, and `_luxa._tcp` with a `mac` TXT record, which is what Luxa
//! clients browse for.

use core::convert::Infallible;
use core::fmt::Write as _;
use core::net::{Ipv4Addr, Ipv6Addr};

use edge_mdns::buf::VecBufAccess;
use edge_mdns::domain::base::Ttl;
use edge_mdns::host::{Host, Service, ServiceAnswers};
use edge_mdns::io::{self, IPV4_DEFAULT_SOCKET, MdnsIoError};
use edge_mdns::{HostAnswersMdnsHandler, NoHostAnswers};
use edge_nal::UdpSplit;
use edge_nal_embassy::{Udp, UdpBuffers, UdpError};
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use esp_println::println;
use heapless::String;
use rand_core::TryRng;
use static_cell::StaticCell;

use crate::config::{HOSTNAME_PREFIX, HTTP_PORT};
use crate::device;

/// Largest mDNS packet sent or received.
const PACKET_BYTES: usize = 1500;

/// How long peers may cache the answers.
const TTL_SECS: u32 = 120;

/// How long to wait before restarting a responder that failed.
const RETRY_SECS: u64 = 5;

/// Answers mDNS queries for the lifetime of the device.
///
/// Spawn once the stack has an address: the host record carries it.
#[embassy_executor::task]
pub async fn advertise(stack: Stack<'static>) {
    let Some(config) = stack.config_v4() else {
        println!("mdns: no address, not advertising");
        return;
    };
    let address = Ipv4Addr::from(config.address.address().octets());

    let mac = device::mac();
    let mut hostname: String<16> = String::new();
    let _ = write!(
        hostname,
        "{HOSTNAME_PREFIX}-{:02x}{:02x}{:02x}",
        mac[3], mac[4], mac[5]
    );
    let mut mac_text: String<12> = String::new();
    for byte in mac {
        let _ = write!(mac_text, "{byte:02x}");
    }

    let host = Host {
        hostname: &hostname,
        ipv4: address,
        ipv6: Ipv6Addr::UNSPECIFIED,
        ttl: Ttl::from_secs(TTL_SECS),
    };
    let http = Service {
        name: &hostname,
        priority: 0,
        weight: 0,
        service: "_http",
        protocol: "_tcp",
        port: HTTP_PORT,
        service_subtypes: &[],
        txt_kvs: &[],
    };
    let txt = [("mac", mac_text.as_str())];
    let api = Service {
        service: "_luxa",
        txt_kvs: &txt,
        ..http.clone()
    };
    let answers = NoHostAnswers
        .chain(ServiceAnswers::new(&host, &http))
        .chain(ServiceAnswers::new(&host, &api));

    static BUFFERS: StaticCell<UdpBuffers<1, PACKET_BYTES, PACKET_BYTES, 2>> = StaticCell::new();
    let udp = Udp::new(stack, BUFFERS.init(UdpBuffers::new()));
    let recv_buf = VecBufAccess::<NoopRawMutex, PACKET_BYTES>::new();
    let send_buf = VecBufAccess::<NoopRawMutex, PACKET_BYTES>::new();
    let changed = Signal::<NoopRawMutex, ()>::new();

    println!("mdns: advertising {hostname}.local");
    loop {
        let result = async {
            let mut socket =
                io::bind(&udp, IPV4_DEFAULT_SOCKET, Some(Ipv4Addr::UNSPECIFIED), None).await?;
            let (recv, send) = socket.split();
            let mdns = io::Mdns::new(
                Some(Ipv4Addr::UNSPECIFIED),
                None,
                recv,
                send,
                &recv_buf,
                &send_buf,
                HardwareRng(esp_hal::rng::Rng::new()),
                &changed,
            );
            mdns.run(HostAnswersMdnsHandler::new(&answers)).await
        };
        let error: MdnsIoError<UdpError> = match result.await {
            Ok(()) => continue,
            Err(error) => error,
        };
        println!("mdns: {error:?}, restarting");
        Timer::after_secs(RETRY_SECS).await;
    }
}

/// The hardware RNG, for the random delays mDNS puts before answers.
struct HardwareRng(esp_hal::rng::Rng);

impl TryRng for HardwareRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(self.0.random())
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok((u64::from(self.0.random()) << 32) | u64::from(self.0.random()))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        self.0.read(dst);
        Ok(())
    }
}
