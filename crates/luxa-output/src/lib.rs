//! The output stage: the last thing that touches pixel values.
//!
//! After the compositor has rendered a frame, this crate applies the global,
//! whole-fixture properties that no effect should know about — gamma, then
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
//! This stage takes plain values in [`Output`], not the engine's state type, so
//! any renderer can use it without depending on Luxa's control vocabulary.
//! Nothing is lost: "off" needs no interpretation here, because in the state
//! model being off *is* brightness zero. As more fixture-wide settings arrive —
//! a current limiter, colour temperature — they join that type, and the runtime
//! maps state onto it.

#![no_std]
#![forbid(unsafe_code)]

use luxa_color::{Crgb, nscale8_video};

/// What a channel value becomes so that the strip *looks* as bright as the
/// value asks for: `round(255 · (value / 255)^2.2)`.
///
/// LEDs emit in proportion to their drive, while the eye reads brightness on a
/// curve much closer to this one. Without it a channel at half scale looks far
/// brighter than half, and the low end wastes most of its range. The exponent
/// and the rounding match what widely used LED controller firmware applies by
/// default, so a colour set through the API shows the same shade there and
/// here — and the gamma-compensated palettes in `color8` land back on their
/// originals once this is applied.
pub const GAMMA_2_2: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 11, 11,
    11, 12, 12, 13, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 22, 22, 23,
    23, 24, 25, 25, 26, 26, 27, 28, 28, 29, 30, 30, 31, 32, 33, 33, 34, 35, 35, 36, 37, 38, 39, 39,
    40, 41, 42, 43, 43, 44, 45, 46, 47, 48, 49, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61,
    62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 73, 74, 75, 76, 77, 78, 79, 81, 82, 83, 84, 85, 87, 88,
    89, 90, 91, 93, 94, 95, 97, 98, 99, 100, 102, 103, 105, 106, 107, 109, 110, 111, 113, 114, 116,
    117, 119, 120, 121, 123, 124, 126, 127, 129, 130, 132, 133, 135, 137, 138, 140, 141, 143, 145,
    146, 148, 149, 151, 153, 154, 156, 158, 159, 161, 163, 165, 166, 168, 170, 172, 173, 175, 177,
    179, 181, 182, 184, 186, 188, 190, 192, 194, 196, 197, 199, 201, 203, 205, 207, 209, 211, 213,
    215, 217, 219, 221, 223, 225, 227, 229, 231, 234, 236, 238, 240, 242, 244, 246, 248, 251, 253,
    255,
];

/// Corrects every pixel for the eye, in place.
///
/// Apply it to a finished frame, before brightness: brightness is a property
/// of the fixture, and correcting after it would bend the fade instead of the
/// colours.
pub fn apply_gamma(pixels: &mut [Crgb]) {
    for pixel in pixels.iter_mut() {
        pixel.r = GAMMA_2_2[usize::from(pixel.r)];
        pixel.g = GAMMA_2_2[usize::from(pixel.g)];
        pixel.b = GAMMA_2_2[usize::from(pixel.b)];
    }
}

/// What the output stage does to a finished frame.
///
/// More fixture-wide settings will join it, so build one with [`new`](Self::new)
/// rather than by naming every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Output {
    /// Fixture brightness; `0` is off.
    pub brightness: u8,
    /// Correct colours for the eye.
    pub gamma: bool,
}

impl Output {
    /// `brightness`, with gamma on — what a fixture shows by default.
    pub const fn new(brightness: u8) -> Self {
        Self {
            brightness,
            gamma: true,
        }
    }

    /// With gamma on or off. Off sends what the effects drew, which is what a
    /// strip that corrects for itself wants.
    pub const fn gamma(mut self, gamma: bool) -> Self {
        self.gamma = gamma;
        self
    }
}

/// Finishes a rendered frame for the wire: gamma, then brightness.
///
/// Call it once per frame, after the compositor has rendered and before the
/// wire encoder runs.
pub fn finish(pixels: &mut [Crgb], output: Output) {
    if output.gamma {
        apply_gamma(pixels);
    }
    apply_brightness(pixels, output.brightness);
}

/// Scales every pixel by `brightness`, where `255` is unattenuated and `0` is
/// exactly black.
///
/// Scaling is video-style: a channel an effect lit stays lit at every
/// brightness above zero, so turning the fixture down dims the whole frame
/// instead of dropping its faintest pixels out one by one. Zero is the
/// exception and is exactly black, because in the state model being off *is*
/// brightness zero.
///
/// Prefer [`finish`], which applies gamma first.
pub fn apply_brightness(pixels: &mut [Crgb], brightness: u8) {
    if brightness == u8::MAX {
        return;
    }
    nscale8_video(pixels, brightness);
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
    fn dimming_keeps_a_lit_pixel_lit_until_it_is_off() {
        let mut px = [Crgb::new(8, 0, 0)];
        apply_brightness(&mut px, 1);
        assert_eq!(
            px[0].r, 1,
            "a lit channel survives any brightness above zero"
        );
        apply_brightness(&mut px, 0);
        assert!(px[0].is_black(), "zero is off, and off is black");
    }

    #[test]
    fn the_gamma_table_is_the_2_2_curve() {
        assert_eq!(GAMMA_2_2[0], 0, "black stays black");
        assert_eq!(GAMMA_2_2[255], 255, "full stays full");
        assert_eq!(GAMMA_2_2[128], 56, "half scale looks far dimmer than half");
        assert!(
            GAMMA_2_2.windows(2).all(|pair| pair[0] <= pair[1]),
            "a higher value never shows dimmer"
        );
        for (value, corrected) in GAMMA_2_2.iter().enumerate() {
            let want = libm::pow(value as f64 / 255.0, 2.2) * 255.0 + 0.5;
            assert_eq!(u32::from(*corrected), want as u32, "value {value}");
        }
    }

    #[test]
    fn gamma_corrects_a_frame_in_place() {
        let mut px = [Crgb::new(255, 128, 0), Crgb::new(64, 32, 16)];
        apply_gamma(&mut px);
        assert_eq!(px[0], Crgb::new(255, 56, 0));
        assert_eq!(
            px[1],
            Crgb::new(GAMMA_2_2[64], GAMMA_2_2[32], GAMMA_2_2[16])
        );
    }

    #[test]
    fn finishing_applies_gamma_before_brightness() {
        let frame = [Crgb::new(255, 128, 0)];

        let mut finished = frame;
        finish(&mut finished, Output::new(128));

        let mut by_hand = frame;
        apply_gamma(&mut by_hand);
        apply_brightness(&mut by_hand, 128);
        assert_eq!(finished, by_hand);

        let mut other_way = frame;
        apply_brightness(&mut other_way, 128);
        apply_gamma(&mut other_way);
        assert_ne!(finished, other_way, "the order is part of the look");
    }

    #[test]
    fn gamma_can_be_switched_off() {
        let mut px = [Crgb::new(255, 128, 0)];
        finish(&mut px, Output::new(255).gamma(false));
        assert_eq!(px, [Crgb::new(255, 128, 0)], "exactly what was drawn");
    }

    #[test]
    fn empty_frame_is_a_no_op() {
        apply_brightness(&mut [], 128);
        apply_gamma(&mut []);
        finish(&mut [], Output::new(128));
    }
}
