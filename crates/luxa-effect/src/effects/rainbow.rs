//! A rainbow scrolling along the strip.

use luxa_color::{Chsv, ColorBlend, Crgb, color_from_palette16, hsv2rgb_rainbow};

use crate::{Ctx, Effect, Params};

/// A hue wheel spread along the view, scrolling over time.
///
/// Speed sets how fast the wheel scrolls. Intensity sets how much of the wheel
/// fits along the view: from a sixteenth up to sixteen wheels, doubling every
/// 29 steps, with the middle setting showing exactly one. A palette takes the
/// place of the wheel.
///
/// The effect keeps no state, so a frame is a pure function of the clock, the
/// view length and the params — frame *n* is identical however you got there.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rainbow;

impl Rainbow {
    /// The rainbow effect.
    pub const fn new() -> Self {
        Self
    }

    /// The hue at the start of the view for a frame.
    ///
    /// Scrolling is a 16-bit phase advanced by `(speed / 4 + 2)` per
    /// millisecond. Because the phase is taken modulo 2¹⁶, which divides 2³²,
    /// the scroll stays continuous across the clock's wrap.
    fn offset(ctx: &Ctx, speed: u8) -> u8 {
        let rate = u32::from(speed >> 2) + 2;
        ((ctx.now_ms().wrapping_mul(rate) & 0xFFFF) >> 8) as u8
    }
}

impl Effect for Rainbow {
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx, params: &Params) {
        let len = view.len();
        if len == 0 {
            return;
        }
        let offset = Self::offset(ctx, params.speed);
        // 16 hue steps at intensity 0, doubling every 29: 256 is one full wheel.
        let span = 16usize << (params.intensity / 29);
        for (i, pixel) in view.iter_mut().enumerate() {
            let hue = ((i * span / len) as u8).wrapping_add(offset);
            *pixel = match &params.palette {
                Some(palette) => color_from_palette16(palette, hue, 255, ColorBlend::LinearBlend),
                None => hsv2rgb_rainbow(Chsv::new(hue, 255, 255)),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(len: usize, now_ms: u32, params: &Params) -> [Crgb; 16] {
        let mut buf = [Crgb::new(0, 0, 0); 16];
        Rainbow.render(&mut buf[..len], &Ctx::from_millis(now_ms), params);
        buf
    }

    fn hue(h: u8) -> Crgb {
        hsv2rgb_rainbow(Chsv::new(h, 255, 255))
    }

    #[test]
    fn fills_every_pixel_with_a_lit_colour() {
        assert!(
            render(16, 0, &Params::DEFAULT)
                .iter()
                .all(|p| !p.is_black())
        );
    }

    #[test]
    fn is_a_pure_function_of_its_inputs() {
        let a = render(16, 1234, &Params::DEFAULT);
        let mut used = Rainbow;
        let mut scratch = [Crgb::new(0, 0, 0); 16];
        used.render(&mut scratch, &Ctx::from_millis(99), &Params::DEFAULT);
        let mut b = [Crgb::new(0, 0, 0); 16];
        used.render(&mut b, &Ctx::from_millis(1234), &Params::DEFAULT);
        assert_eq!(a, b, "no hidden state");
    }

    #[test]
    fn scrolls_over_time() {
        assert_ne!(
            render(16, 0, &Params::DEFAULT),
            render(16, 500, &Params::DEFAULT)
        );
    }

    #[test]
    fn speed_sets_the_scroll_rate() {
        let slow = Params {
            speed: 0,
            ..Params::DEFAULT
        };
        let fast = Params {
            speed: 255,
            ..Params::DEFAULT
        };
        // After one second the slow wheel has moved 2000/256 steps, the fast
        // one 65000/256 (mod 256).
        assert_eq!(Rainbow::offset(&Ctx::from_millis(1000), 0), 7);
        assert_eq!(Rainbow::offset(&Ctx::from_millis(1000), 255), 253);
        assert_ne!(render(4, 1000, &slow), render(4, 1000, &fast));
    }

    #[test]
    fn the_middle_intensity_spreads_one_full_wheel() {
        // 4 pixels at time 0 => hues 0, 64, 128, 192.
        let out = render(4, 0, &Params::DEFAULT);
        assert_eq!(out[..4], [hue(0), hue(64), hue(128), hue(192)]);
    }

    #[test]
    fn low_intensity_spreads_a_sixteenth_of_the_wheel() {
        let narrow = Params {
            intensity: 0,
            ..Params::DEFAULT
        };
        // 16 pixels, 16 hue steps: one step per pixel.
        let out = render(16, 0, &narrow);
        assert_eq!(out[15], hue(15));
    }

    #[test]
    fn a_palette_takes_the_place_of_the_wheel() {
        let lava = Params {
            palette: Some(luxa_color::LAVA_COLORS),
            ..Params::DEFAULT
        };
        let at =
            |h| color_from_palette16(&luxa_color::LAVA_COLORS, h, 255, ColorBlend::LinearBlend);
        assert_eq!(render(4, 0, &lava)[..4], [at(0), at(64), at(128), at(192)]);
    }

    #[test]
    fn hue_stays_continuous_across_the_clock_wrap() {
        let before = Rainbow::offset(&Ctx::from_millis(u32::MAX), 128);
        let after = Rainbow::offset(&Ctx::from_millis(0), 128);
        assert_eq!(after, before.wrapping_add(1));
    }

    #[test]
    fn empty_and_single_pixel_views() {
        Rainbow.render(&mut [], &Ctx::from_millis(0), &Params::DEFAULT);
        assert_eq!(render(1, 0, &Params::DEFAULT)[0], hue(0));
    }
}
