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
/// Deep enough that a dragged slider does not block its HTTP handler, shallow
/// enough that a wedged engine applies backpressure instead of hoarding stale
/// intent.
pub const COMMAND_QUEUE_DEPTH: usize = 16;

/// TCP port for the control UI.
pub const HTTP_PORT: u16 = 80;

/// Wokwi's built-in gateway. Open network, always on channel 6 — using it
/// means the demo is a link somebody can click, with no credentials in the
/// repository.
pub const WIFI_SSID: &str = "Wokwi-GUEST";
/// Wokwi's gateway is open; no password.
pub const WIFI_PASSWORD: &str = "";
