//! WS2812 sink for the ESP32 RMT peripheral.
//!
//! This crate is the isolation boundary — the one place in Luxa that touches
//! silicon. It is *supposed* to be replaced wholesale when the target changes:
//! an RP2350 port is a new crate implementing the same shape with PIO, and
//! nothing above it moves.
//!
//! Note what it does **not** contain. No color math, no brightness, no channel
//! order, no notion of what a "pixel" is. It receives a byte stream and a
//! [`BitTiming`] from `luxa-wire` and its whole job is turning that pair into
//! RMT pulse codes and clocking them out. If you ever find yourself wanting to
//! scale a value here, it belongs in `luxa-output`.
//!
//! # How the encoding works
//!
//! The RMT peripheral transmits a list of [`PulseCode`]s, each describing two
//! consecutive line levels with a duration in peripheral ticks. WS2812's line
//! code is one symbol per bit — high for a while, then low, with the ratio
//! carrying the value — so one bit maps to exactly one pulse code, and the
//! whole frame is a flat array with no bit-banging and no timing jitter from
//! the CPU.

#![no_std]

use esp_hal::Async;
use esp_hal::gpio::Level;
use esp_hal::rmt::{Channel, Error as RmtError, PulseCode, Tx};
use esp_hal::time::Rate;
use luxa_wire::{BitOrder, BitTiming, Ws2812};

/// Pulse codes needed to transmit `pixel_count` WS2812 pixels.
///
/// One code per bit, plus one trailing code that holds the line low for the
/// latch interval and terminates the transmission.
pub const fn codes_for(pixel_count: usize) -> usize {
    Ws2812::buffer_len(pixel_count) * 8 + 1
}

/// Something went wrong sinking a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// The frame needs more pulse codes than this driver was sized for.
    ///
    /// `CODES` is a const parameter, so this can only happen if the caller
    /// passes a byte slice longer than the strip the driver was built for.
    FrameTooLong {
        /// Codes the frame needs.
        needed: usize,
        /// Codes this driver has room for.
        capacity: usize,
    },
    /// A timing value does not fit in an RMT pulse code at the configured
    /// clock rate. Lower the RMT frequency or shorten the reset interval.
    TimingOutOfRange,
    /// The peripheral reported a transmission failure.
    Transmission,
}

impl From<RmtError> for DriverError {
    fn from(_: RmtError) -> Self {
        Self::Transmission
    }
}

/// Drives a WS2812 strip from an RMT TX channel.
///
/// `CODES` sizes the pulse-code buffer; build it with [`codes_for`]:
///
/// ```ignore
/// const LEDS: usize = 60;
/// let mut strip: RmtWs2812<{ codes_for(LEDS) }> = RmtWs2812::new(channel, clock, Ws2812::TIMING)?;
/// ```
pub struct RmtWs2812<'d, const CODES: usize> {
    channel: Channel<'d, Async, Tx>,
    /// The symbol for a `0` bit, pre-computed once.
    zero: PulseCode,
    /// The symbol for a `1` bit, pre-computed once.
    one: PulseCode,
    /// Trailing low period that latches the frame. A zero second-length makes
    /// this the end marker too, so the line stops exactly where we want it.
    latch: PulseCode,
    codes: [PulseCode; CODES],
}

impl<'d, const CODES: usize> RmtWs2812<'d, CODES> {
    /// Builds a driver for `channel`, clocked at `clock`.
    ///
    /// `clock` must be the rate the [`Rmt`](esp_hal::rmt::Rmt) was created
    /// with, and the channel must be configured with a divider of 1 — that is
    /// the tick rate the nanosecond timings are converted against. Passing the
    /// timing as data rather than baking it in is deliberate: a marginal strip
    /// gets a tweaked profile, not a patched driver.
    pub fn new(
        channel: Channel<'d, Async, Tx>,
        clock: Rate,
        timing: BitTiming,
    ) -> Result<Self, DriverError> {
        // WS2812 clocks MSB first; if a chipset ever wants the other order the
        // bit loop below has to change, so refuse rather than emit garbage.
        assert!(
            matches!(Ws2812::BIT_ORDER, BitOrder::MsbFirst),
            "this driver only implements MSB-first bit order"
        );

        let mhz = clock.as_mhz();
        let ns_to_ticks = |ns: u16| -> Option<u16> {
            // ticks = ns * MHz / 1000, rounded to nearest.
            let ticks = (ns as u32 * mhz + 500) / 1_000;
            u16::try_from(ticks)
                .ok()
                .filter(|t| *t <= PulseCode::MAX_LEN)
        };

        let t0h = ns_to_ticks(timing.t0h_ns).ok_or(DriverError::TimingOutOfRange)?;
        let t0l = ns_to_ticks(timing.t0l_ns).ok_or(DriverError::TimingOutOfRange)?;
        let t1h = ns_to_ticks(timing.t1h_ns).ok_or(DriverError::TimingOutOfRange)?;
        let t1l = ns_to_ticks(timing.t1l_ns).ok_or(DriverError::TimingOutOfRange)?;

        let latch_ticks = u16::try_from(timing.reset_us as u32 * mhz)
            .ok()
            .filter(|t| *t <= PulseCode::MAX_LEN)
            .ok_or(DriverError::TimingOutOfRange)?;

        // A zero second-length marks the end of the transmission, so this one
        // code both latches the frame and stops the channel.
        let latch = PulseCode::new(Level::Low, latch_ticks, Level::Low, 0);

        Ok(Self {
            channel,
            zero: PulseCode::new(Level::High, t0h, Level::Low, t0l),
            one: PulseCode::new(Level::High, t1h, Level::Low, t1l),
            latch,
            codes: [latch; CODES],
        })
    }

    /// Clocks one encoded frame out to the strip.
    ///
    /// `bytes` is exactly what [`Ws2812::encode`] produced — already in the
    /// strip's channel order, already brightness-scaled. This method does not
    /// interpret it.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), DriverError> {
        let needed = bytes.len() * 8 + 1;
        if needed > CODES {
            return Err(DriverError::FrameTooLong {
                needed,
                capacity: CODES,
            });
        }

        let mut i = 0;
        for byte in bytes {
            for bit in (0..8).rev() {
                self.codes[i] = if byte & (1 << bit) != 0 {
                    self.one
                } else {
                    self.zero
                };
                i += 1;
            }
        }
        self.codes[i] = self.latch;

        self.channel.transmit(&self.codes[..=i]).await?;
        Ok(())
    }
}
