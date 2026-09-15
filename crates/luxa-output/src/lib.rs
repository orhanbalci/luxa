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

/// Fixture brightness as shown, fading linearly toward a target.
///
/// Call [`update`](Self::update) once per frame with the brightness the
/// fixture should reach, how long the change into it takes, and the frame's
/// clock, then pass the result to [`apply_brightness`]. A new target starts a
/// fresh fade from whatever is shown at that moment, so a change arriving
/// mid-fade carries on smoothly instead of jumping. The duration is taken when
/// the target changes, so a one-shot duration shapes only its own change.
///
/// Switching off is a fade to `0`, and switching on a fade back up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrightnessFade {
    from: u8,
    to: u8,
    start_ms: u32,
    duration_ms: u16,
}

impl BrightnessFade {
    /// A fade already at `brightness`.
    pub const fn new(brightness: u8) -> Self {
        Self {
            from: brightness,
            to: brightness,
            start_ms: 0,
            duration_ms: 0,
        }
    }

    /// The brightness to show at `now_ms`, first starting a fade toward
    /// `target` over `duration_ms` if the target changed.
    pub fn update(&mut self, target: u8, duration_ms: u16, now_ms: u32) -> u8 {
        if target != self.to {
            self.from = self.at(now_ms);
            self.to = target;
            self.start_ms = now_ms;
            self.duration_ms = duration_ms;
        }
        self.at(now_ms)
    }

    /// The brightness shown at `now_ms`, without changing the target.
    ///
    /// `now_ms` is the wrapping animation clock; the fade keeps working across
    /// its wrap.
    pub fn at(&self, now_ms: u32) -> u8 {
        let elapsed = now_ms.wrapping_sub(self.start_ms);
        let duration = u32::from(self.duration_ms);
        if elapsed >= duration {
            return self.to;
        }
        // `elapsed < duration <= u16::MAX`, so this fits an i32.
        let from = i32::from(self.from);
        let span = i32::from(self.to) - from;
        (from + span * elapsed as i32 / duration as i32) as u8
    }

    /// The brightness being faded toward.
    pub const fn target(&self) -> u8 {
        self.to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_fade_shows_its_brightness_at_once() {
        let fade = BrightnessFade::new(90);
        assert_eq!(fade.at(0), 90);
        assert_eq!(fade.at(123_456), 90);
    }

    #[test]
    fn a_fade_is_linear_in_time() {
        let mut fade = BrightnessFade::new(0);
        assert_eq!(fade.update(200, 1_000, 5_000), 0, "starts where it was");
        assert_eq!(fade.at(5_250), 50);
        assert_eq!(fade.at(5_500), 100);
        assert_eq!(fade.at(6_000), 200);
        assert_eq!(fade.at(9_000), 200, "and stays");
    }

    #[test]
    fn fading_down_works_the_same_way() {
        let mut fade = BrightnessFade::new(200);
        fade.update(0, 1_000, 0);
        assert_eq!(fade.at(500), 100);
        assert_eq!(fade.at(1_000), 0, "off is exactly zero");
    }

    #[test]
    fn a_zero_duration_changes_at_once() {
        let mut fade = BrightnessFade::new(10);
        assert_eq!(fade.update(250, 0, 77), 250);
    }

    #[test]
    fn a_new_target_mid_fade_starts_from_what_is_shown() {
        let mut fade = BrightnessFade::new(0);
        fade.update(200, 1_000, 0);
        assert_eq!(fade.update(0, 1_000, 500), 100, "no jump");
        assert_eq!(fade.at(1_000), 50);
        assert_eq!(fade.at(1_500), 0);
    }

    #[test]
    fn repeating_the_target_does_not_restart_the_fade() {
        let mut fade = BrightnessFade::new(0);
        fade.update(200, 1_000, 0);
        assert_eq!(fade.update(200, 1_000, 500), 100);
        assert_eq!(
            fade.update(200, 0, 750),
            150,
            "its duration was fixed at the start"
        );
        assert_eq!(fade.target(), 200);
    }

    #[test]
    fn a_fade_continues_across_the_clock_wrap() {
        let mut fade = BrightnessFade::new(0);
        fade.update(100, 1_000, u32::MAX - 499);
        assert_eq!(fade.at(0), 50);
        assert_eq!(fade.at(500), 100);
    }

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
