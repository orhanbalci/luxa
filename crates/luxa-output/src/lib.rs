//! The output stage: the last thing that touches pixel values.
//!
//! After the compositor has rendered a frame, this crate applies the global,
//! whole-fixture properties that no effect should know about — today,
//! brightness. It runs on a finished canvas and produces the values that go on
//! the wire.
//!
//! # Why brightness lives here and not in the driver
//!
//! It is genuinely tempting to fold a `* brightness` into the driver's write
//! loop, or into the render task, because with one effect and one strip that
//! is fewer lines. Apply the relocation test and it collapses: swap WS2812 for
//! APA102, or ESP32 for RP2350, and does "the user asked for half
//! brightness" change meaning? No. So it cannot live in the driver — a second
//! driver would have to reimplement it, and the two would drift.
//!
//! The same test is why it is not in the runtime: swap HTTP for MQTT and
//! brightness still means the same thing. The runtime's job is to *call* this,
//! not to be it.
//!
//! # Why it takes plain values
//!
//! This stage takes a brightness, not the engine's state type, so any renderer
//! can use it without depending on Luxa's control vocabulary. Nothing is lost:
//! "off" needs no interpretation here, because in the state model being off
//! *is* brightness zero. As fixture-wide output settings arrive — a current
//! limiter, colour temperature, gamma — they come as a settings type defined in
//! this crate, and the runtime maps state onto it.

#![no_std]
#![forbid(unsafe_code)]

use luxa_color::{Crgb, nscale8};

/// Scales every pixel by `brightness`, where `255` is unattenuated and `0` is
/// exactly black.
///
/// This is plain (non-video) scaling: a dim pixel is allowed to reach true
/// black as brightness falls, which is what you want for a global fade.
/// Video-style scaling — which never lets a lit pixel go fully dark — is an
/// effect-level concern, not a fixture-level one.
///
/// Call it once per frame, after the compositor has rendered and before the
/// wire encoder runs.
pub fn apply_brightness(pixels: &mut [Crgb], brightness: u8) {
    if brightness == u8::MAX {
        return;
    }
    nscale8(pixels, brightness);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> [Crgb; 3] {
        [
            Crgb::new(255, 255, 255),
            Crgb::new(200, 100, 50),
            Crgb::new(1, 0, 0),
        ]
    }

    #[test]
    fn full_brightness_is_the_identity() {
        let mut px = frame();
        apply_brightness(&mut px, 255);
        assert_eq!(px, frame(), "255 must not dim the frame at all");
    }

    #[test]
    fn brightness_zero_blacks_the_frame_out() {
        let mut px = frame();
        apply_brightness(&mut px, 0);
        assert!(
            px.iter().all(Crgb::is_black),
            "zero — which is what 'off' is — must be exactly black"
        );
    }

    #[test]
    fn half_brightness_roughly_halves_each_channel() {
        let mut px = frame();
        apply_brightness(&mut px, 128);
        // Integer scaling, so allow the off-by-one the fixed-point form gives.
        for (before, after) in frame().iter().zip(px.iter()) {
            for (b, a) in [
                (before.r, after.r),
                (before.g, after.g),
                (before.b, after.b),
            ] {
                let want = (b as u16).div_ceil(2);
                assert!(
                    (a as i32 - want as i32).abs() <= 1,
                    "scaling {b} by 128 gave {a}, expected about {want}"
                );
            }
        }
    }

    #[test]
    fn brightness_is_monotonic() {
        let mut prev = 0u32;
        for brightness in [0u8, 1, 32, 64, 128, 200, 255] {
            let mut px = frame();
            apply_brightness(&mut px, brightness);
            let sum: u32 = px
                .iter()
                .map(|p| p.r as u32 + p.g as u32 + p.b as u32)
                .sum();
            assert!(
                sum >= prev,
                "brightness {brightness} produced less light than the step below it"
            );
            prev = sum;
        }
    }

    #[test]
    fn empty_frame_is_a_no_op() {
        apply_brightness(&mut [], 128);
    }
}
