//! Deployment parameters for this board.
//!
//! Everything hardware-derived lives here as a value, not as a constant buried
//! in a driver. That is the rule the whole project is built on: the day a real
//! strip shows up and the timing needs a nudge, or the clone turns out to be
//! RGB rather than GRB, the change is in this file — not in `luxa-wire`, and
//! certainly not in the driver.

use luxa_color::ColorOrder;
use luxa_msg::Catalogue;
use luxa_wire::{BitTiming, Ws2812};

/// How the attached strip behaves.
///
/// Slice 1 hardcodes one profile because there is one strip and no config
/// persistence yet. The *shape* is already right, so the slice that adds
/// persistence loads this struct instead of introducing it.
pub struct LedProfile {
    /// Number of LEDs physically on the strip.
    pub pixel_count: usize,
    /// The channel order this strip expects.
    pub color_order: ColorOrder,
    /// Line timing for the strip's chipset.
    pub timing: BitTiming,
}

/// The strip Wokwi simulates: 60 pixels of stock WS2812B.
pub const PROFILE: LedProfile = LedProfile {
    pixel_count: LEDS,
    color_order: ColorOrder::Grb,
    timing: Ws2812::TIMING,
};

/// Canvas length, in pixels.
///
/// A `const` because the framebuffer is stack-allocated; [`LedProfile`]'s
/// `pixel_count` may be shorter, in which case the tail is simply not sent.
pub const LEDS: usize = 60;

/// Pixels the compositor keeps for segments' own frames between renders.
///
/// Effects draw over what they drew last, so every segment needs memory that
/// outlives the frame. Twice the canvas lets overlapping segments keep theirs
/// too; a segment that finds the pool full is not drawn.
pub const COMPOSITOR_PIXELS: usize = 2 * LEDS;

/// Most segments the fixture state can hold.
///
/// 32 is what ESP32-class controllers without PSRAM offer, and clients read it
/// back, so matching it keeps their segment editors honest. At this size the
/// whole published state is roughly 3 KB.
pub const MAX_SEGMENTS: usize = 32;

/// Byte capacity of a segment name.
pub const SEGMENT_NAME_LEN: usize = 64;

/// The effect and palette ids the engine accepts: exactly what the renderer
/// can draw.
pub const CATALOGUE: Catalogue = luxa_segment::CATALOGUE;

/// RMT source clock. With a divider of 1 this gives 12.5 ns per tick, fine
/// enough to hit WS2812's 400/800 ns pulses within a few percent.
pub const RMT_CLOCK_MHZ: u32 = 80;

/// Frame interval. 16 ms is ~62 fps — comfortably above the point where the
/// scroll looks continuous, and far longer than the ~1.8 ms a 60-pixel frame
/// takes to clock out.
pub const FRAME_MS: u64 = 16;

/// Depth of the command channel.
///
/// One request becomes at most a global command, one command per segment and
/// a compaction, and it is queued whole or not at all. Room for two of the
/// largest keeps a dragged slider from being refused, while a wedged engine
/// still applies backpressure instead of hoarding stale intent.
pub const COMMAND_QUEUE_DEPTH: usize = 2 * (MAX_SEGMENTS + 2);

/// How long a request waits for the engine to apply its commands before
/// answering with whatever state is published.
pub const APPLY_TIMEOUT_MS: u64 = 1000;

/// Largest request accepted: an HTTP body, or a WebSocket message.
///
/// The reference caps WebSocket messages at 1428 bytes; every request a
/// client sends to change state fits within this. Each web task holds a
/// buffer of this size, so it is not raised lightly.
pub const MAX_REQUEST_BYTES: usize = 1536;

/// Concurrent WebSocket clients. Each holds a web task for as long as it is
/// connected.
pub const WEBSOCKET_CLIENTS: usize = 2;

/// TCP port for the control UI and the API.
pub const HTTP_PORT: u16 = 80;

/// The fixture's name, as clients display it.
pub const DEVICE_NAME: &str = "Luxa";

/// The start of the fixture's mDNS hostname; the tail of the MAC address
/// follows, as in `luxa-a1b2c3.local`.
pub const HOSTNAME_PREFIX: &str = "luxa";

/// Brand and product reported in the info document.
pub const BRAND: &str = "Luxa";
/// See [`BRAND`].
pub const PRODUCT: &str = "Luxa";

/// The API version reported to clients, which gate features on it. This is
/// the reference release whose API Luxa implements, not Luxa's own version.
pub const API_VERSION: &str = "0.15.0";
/// [`API_VERSION`] as the number clients compare.
pub const API_VERSION_ID: u32 = 2_412_100;

/// Wokwi's built-in gateway. Open network, always on channel 6 — using it
/// means the demo is a link somebody can click, with no credentials in the
/// repository.
pub const WIFI_SSID: &str = "Wokwi-GUEST";
/// Wokwi's gateway is open; no password.
pub const WIFI_PASSWORD: &str = "";
