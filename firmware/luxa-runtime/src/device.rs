//! What the API reports about this device: catalogue names and the facts in
//! the info document.

use core::cell::Cell;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::String;
use luxa_api::json::{Info, Names, Wifi};
use luxa_effect::{EffectKind, PALETTES};

use crate::config::{
    API_VERSION, API_VERSION_ID, BRAND, CATALOGUE, DEVICE_NAME, FRAME_MS, LEDS, POWER_SUPPLY_MA,
    PRODUCT,
};
use crate::websocket;

/// The station MAC address, recorded when the radio comes up.
static MAC: Mutex<CriticalSectionRawMutex, Cell<[u8; 6]>> = Mutex::new(Cell::new([0; 6]));

/// The IPv4 address, recorded once DHCP assigns one.
static ADDRESS: Mutex<CriticalSectionRawMutex, Cell<Option<[u8; 4]>>> = Mutex::new(Cell::new(None));

/// Records the station MAC address.
pub fn set_mac(mac: [u8; 6]) {
    MAC.lock(|cell| cell.set(mac));
}

/// The station MAC address.
pub fn mac() -> [u8; 6] {
    MAC.lock(Cell::get)
}

/// What the frame last sent draws, in milliamps, as the output stage
/// estimated it.
static POWER_MA: Mutex<CriticalSectionRawMutex, Cell<u32>> = Mutex::new(Cell::new(0));

/// Records what the frame just sent draws.
pub fn set_power_ma(milliamps: u32) {
    POWER_MA.lock(|cell| cell.set(milliamps));
}

/// Records the IPv4 address.
pub fn set_address(octets: [u8; 4]) {
    ADDRESS.lock(|cell| cell.set(Some(octets)));
}

/// The names of the effects and palettes this firmware renders.
pub struct Registry;

impl Names for Registry {
    fn effect_count(&self) -> u16 {
        CATALOGUE.effects.end()
    }

    fn effect_name(&self, id: u8) -> Option<&str> {
        EffectKind::from_id(id).map(|kind| kind.name())
    }

    fn effect_descriptor(&self, id: u8) -> Option<&str> {
        EffectKind::from_id(id).map(|kind| kind.descriptor().as_str())
    }

    fn palette_count(&self) -> u16 {
        CATALOGUE.palettes.end()
    }

    fn palette_name(&self, id: u8) -> Option<&str> {
        PALETTES
            .iter()
            .find(|palette| palette.id == id)
            .map(|palette| palette.name)
    }
}

/// The device facts at one moment, owning the text the info document borrows.
pub struct Facts {
    mac: String<12>,
    ip: String<15>,
    websocket_clients: i32,
    uptime_s: u32,
    free_heap: u32,
    power_ma: u32,
}

impl Facts {
    /// The facts as they are now.
    pub fn now() -> Self {
        let mut mac = String::new();
        for byte in MAC.lock(Cell::get) {
            let _ = write!(mac, "{byte:02x}");
        }
        let mut ip = String::new();
        if let Some([a, b, c, d]) = ADDRESS.lock(Cell::get) {
            let _ = write!(ip, "{a}.{b}.{c}.{d}");
        }
        Self {
            mac,
            ip,
            websocket_clients: i32::from(websocket::clients()),
            uptime_s: embassy_time::Instant::now().as_secs() as u32,
            free_heap: esp_alloc::HEAP.free() as u32,
            power_ma: POWER_MA.lock(Cell::get),
        }
    }

    /// The info document's facts.
    pub fn info(&self) -> Info<'_> {
        Info {
            name: DEVICE_NAME,
            version: API_VERSION,
            version_id: API_VERSION_ID,
            brand: BRAND,
            product: PRODUCT,
            arch: "esp32c3",
            mac: &self.mac,
            ip: &self.ip,
            led_count: LEDS as u16,
            fps: (1000 / FRAME_MS) as u16,
            power_ma: self.power_ma,
            // The limit means nothing without an estimate to weigh against it,
            // so it is reported only once there is one.
            max_power_ma: if self.power_ma > 0 { POWER_SUPPLY_MA } else { 0 },
            // No peer sync yet.
            udp_port: 0,
            websocket_clients: self.websocket_clients,
            uptime_s: self.uptime_s,
            free_heap: self.free_heap,
            options: 0,
            // Signal details live with the Wi-Fi controller and are not
            // gathered yet.
            wifi: Wifi {
                bssid: "",
                rssi: 0,
                signal: 0,
                channel: 0,
                band: "2.4GHz",
                access_point: false,
            },
        }
    }
}
