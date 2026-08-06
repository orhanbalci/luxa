//! Wire framing: turning pixels into the bytes a specific LED chipset expects.
//!
//! This is the chipset boundary. Everything upstream of it — effects, the
//! canvas, the brightness stage — works in plain RGB and has no idea what is
//! on the other end of the data line. Everything downstream of it (the driver)
//! knows about a peripheral but not about color.
//!
//! Swapping WS2812 for APA102 changes this crate and nothing else in the
//! portable tier: the bit layout, the channel order and the timing all live
//! here. Swapping ESP32 for RP2350 changes *none* of it, which is why the
//! crate is pure and `no_std` and every test in it runs on a laptop against
//! known-good byte vectors.
//!
//! # What this crate does not do
//!
//! It does not touch a peripheral, and it does not emit RMT pulse codes, PIO
//! programs or SPI words. It emits the chipset's byte stream plus the
//! [`BitTiming`] those bytes must be clocked at; converting that pair into
//! whatever a given silicon block wants is the driver's job, because that
//! conversion is exactly what changes per platform.

#![no_std]
#![forbid(unsafe_code)]

mod ws2812;

pub use ws2812::Ws2812;

/// The on-wire bit timing a chipset's line code requires, in nanoseconds.
///
/// A driver turns these into whatever its peripheral counts in. They are
/// datasheet numbers, kept as data rather than baked into a driver so that
/// tuning one for a marginal strip never means editing platform code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitTiming {
    /// High time of a `0` bit.
    pub t0h_ns: u16,
    /// Low time of a `0` bit.
    pub t0l_ns: u16,
    /// High time of a `1` bit.
    pub t1h_ns: u16,
    /// Low time of a `1` bit.
    pub t1l_ns: u16,
    /// Line-low time that latches the frame, in microseconds.
    pub reset_us: u16,
}

impl BitTiming {
    /// The full period of one bit, in nanoseconds.
    pub const fn bit_period_ns(&self) -> u32 {
        // Both encodings are the same nominal period; take the longer of the
        // two so a driver sizing a buffer is never short.
        let zero = self.t0h_ns as u32 + self.t0l_ns as u32;
        let one = self.t1h_ns as u32 + self.t1l_ns as u32;
        if one > zero { one } else { zero }
    }
}

/// Bit significance order within each byte of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitOrder {
    /// Bit 7 first — WS2812 and the great majority of clocked LED chipsets.
    MsbFirst,
    /// Bit 0 first.
    LsbFirst,
}

/// Why an encode could not be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The destination buffer cannot hold the encoded frame.
    BufferTooSmall {
        /// Bytes the frame needs.
        needed: usize,
        /// Bytes the caller supplied.
        got: usize,
    },
}

impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BufferTooSmall { needed, got } => {
                write!(f, "wire buffer too small: need {needed} bytes, got {got}")
            }
        }
    }
}

impl core::error::Error for WireError {}
