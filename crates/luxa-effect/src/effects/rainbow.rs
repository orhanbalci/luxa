//! A rainbow scrolling along the strip.

use luxa_color::{Chsv, Crgb, hsv2rgb_rainbow};

use crate::{Ctx, Effect};

/// A full hue wheel spread across the view, scrolling over time.
///
/// This effect is a pure function of [`Ctx::now_ms`] and the view length: it
/// keeps no state, so frame *n* is identical however you got there. That makes
/// it the ideal first effect — every assertion in its tests is about the math,
/// with no history to set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rainbow {
    /// Milliseconds per 1/256th of a hue cycle.
    ///
    /// The full cycle therefore takes `256 * ms_per_hue_step` ms — the default
    /// of 8 gives a shade over two seconds.
    ///
    /// Prefer powers of two. `now_ms` wraps at 2³², and the scrolled hue stays
    /// continuous across that wrap exactly when `256 * ms_per_hue_step`
    /// divides 2³² — otherwise the animation jumps once every ~49.7 days.
    pub ms_per_hue_step: u32,
}

impl Rainbow {
    /// The default scroll rate: a full cycle every 2048 ms.
    pub const DEFAULT_MS_PER_HUE_STEP: u32 = 8;

    /// A rainbow scrolling at the default rate.
    pub const fn new() -> Self {
        Self {
            ms_per_hue_step: Self::DEFAULT_MS_PER_HUE_STEP,
        }
    }

    /// A rainbow whose full cycle takes `ms_per_hue_step * 256` milliseconds.
    ///
    /// A step of `0` is clamped to 1, so the effect degrades to "very fast"
    /// rather than dividing by zero.
    pub const fn with_step(ms_per_hue_step: u32) -> Self {
        Self {
            ms_per_hue_step: if ms_per_hue_step == 0 {
                1
            } else {
                ms_per_hue_step
            },
        }
    }

    /// The hue at the head of the view for a given frame.
    #[inline]
    fn base_hue(&self, ctx: &Ctx) -> u8 {
        let step = if self.ms_per_hue_step == 0 {
            1
        } else {
            self.ms_per_hue_step
        };
        (ctx.now_ms() / step) as u8
    }
}

impl Default for Rainbow {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Rainbow {
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx) {
        let len = view.len();
        if len == 0 {
            return;
        }
        let base = self.base_hue(ctx);
        for (i, pixel) in view.iter_mut().enumerate() {
            // Spread one full wheel over the view: 0..256 exclusive, so the
            // last pixel does not duplicate the first.
            let offset = (i * 256 / len) as u8;
            *pixel = hsv2rgb_rainbow(Chsv::new(base.wrapping_add(offset), 255, 255));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(effect: &mut Rainbow, len: usize, now_ms: u32) -> [Crgb; 16] {
        let mut buf = [Crgb::new(0, 0, 0); 16];
        effect.render(&mut buf[..len], &Ctx::from_millis(now_ms));
        buf
    }

    #[test]
    fn fills_every_pixel_with_a_lit_color() {
        let out = render(&mut Rainbow::new(), 16, 0);
        assert!(out.iter().all(|p| !p.is_black()));
    }

    #[test]
    fn is_a_pure_function_of_time() {
        let a = render(&mut Rainbow::new(), 16, 1234);
        let b = render(&mut Rainbow::new(), 16, 1234);
        assert_eq!(a, b, "same frame must render identically");

        // ...and a fresh instance matches one that has already rendered, i.e.
        // there is genuinely no hidden state.
        let mut used = Rainbow::new();
        render(&mut used, 16, 99);
        assert_eq!(render(&mut used, 16, 1234), a);
    }

    #[test]
    fn scrolls_over_time() {
        let e = Rainbow::new();
        // A full cycle is 256 * 8 = 2048 ms; a quarter of that must differ.
        assert_ne!(
            render(&mut e.clone(), 16, 0),
            render(&mut e.clone(), 16, 512)
        );
    }

    #[test]
    fn a_full_cycle_returns_to_the_start() {
        let cycle_ms = 256 * Rainbow::DEFAULT_MS_PER_HUE_STEP;
        assert_eq!(
            render(&mut Rainbow::new(), 16, 0),
            render(&mut Rainbow::new(), 16, cycle_ms)
        );
    }

    #[test]
    fn hue_stays_continuous_across_the_u32_wrap() {
        // With a power-of-two step the counter rolls over on a cycle boundary,
        // so the frame just before the wrap and the one just after are
        // adjacent, not a jump.
        let e = Rainbow::new();
        let before = e.base_hue(&Ctx::from_millis(u32::MAX));
        let after = e.base_hue(&Ctx::from_millis(0));
        assert_eq!(after, before.wrapping_add(1));
    }

    #[test]
    fn spreads_a_full_wheel_across_the_view() {
        let e = Rainbow::new();
        let ctx = Ctx::from_millis(0);
        let mut buf = [Crgb::new(0, 0, 0); 4];
        e.clone().render(&mut buf, &ctx);
        // 4 pixels => offsets 0, 64, 128, 192.
        for (i, expected_offset) in [0u8, 64, 128, 192].into_iter().enumerate() {
            let want = hsv2rgb_rainbow(Chsv::new(expected_offset, 255, 255));
            assert_eq!(buf[i], want, "pixel {i}");
        }
    }

    #[test]
    fn empty_view_is_a_no_op() {
        Rainbow::new().render(&mut [], &Ctx::from_millis(0));
    }

    #[test]
    fn single_pixel_view_works() {
        let out = render(&mut Rainbow::new(), 1, 0);
        assert_eq!(out[0], hsv2rgb_rainbow(Chsv::new(0, 255, 255)));
    }

    #[test]
    fn zero_step_does_not_divide_by_zero() {
        let mut e = Rainbow { ms_per_hue_step: 0 };
        let mut buf = [Crgb::new(0, 0, 0); 4];
        e.render(&mut buf, &Ctx::from_millis(1000));
        assert_eq!(Rainbow::with_step(0).ms_per_hue_step, 1);
    }
}
