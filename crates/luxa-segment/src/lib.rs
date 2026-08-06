//! The compositor: which effect renders into which pixels.
//!
//! This crate answers one question — given a canvas, who draws where — and it
//! is thin by design in slice 1, not misplaced. The whole strip is segment
//! zero and it runs one effect, so composition is currently "hand the effect
//! the whole view". There is no blending, no transition, no mirroring.
//!
//! It still earns its own crate under the relocation test: swap WS2812 for
//! APA102, or ESP32 for RP2350, or HTTP for MQTT, and "segment zero covers
//! pixels 0..N and runs the rainbow" is unchanged. The layer that grows into
//! multi-segment composition should not have to be extracted from a runtime
//! later; it should be the place that already exists.

#![no_std]
#![forbid(unsafe_code)]

use luxa_color::Crgb;
use luxa_effect::{Ctx, Effect, EffectKind};

/// Renders the configured effects into a canvas.
///
/// Slice 1's compositor is degenerate: one segment covering the entire canvas.
/// ```
/// use luxa_canvas::Canvas;
/// use luxa_effect::Ctx;
/// use luxa_segment::Compositor;
///
/// let mut canvas = Canvas::<8>::black();
/// let mut compositor = Compositor::default();
/// compositor.render(canvas.as_mut_slice(), &Ctx::from_millis(0));
/// assert!(canvas.iter().any(|p| !p.is_black()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Compositor {
    /// The effect running on segment zero.
    ///
    /// Public because selecting it is a later slice's job and will arrive as a
    /// `Command`; there is no invariant here to protect yet.
    pub effect: EffectKind,
}

impl Compositor {
    /// A compositor running `effect` across the whole canvas.
    pub const fn new(effect: EffectKind) -> Self {
        Self { effect }
    }

    /// Renders one frame into `canvas`.
    ///
    /// The canvas is passed whole and handed on whole — that sub-slicing step
    /// is exactly what becomes real when segments become plural.
    pub fn render(&mut self, canvas: &mut [Crgb], ctx: &Ctx) {
        self.effect.render(canvas, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_effect::effects::Rainbow;

    #[test]
    fn renders_the_effect_across_the_whole_canvas() {
        let ctx = Ctx::from_millis(1234);

        let mut composited = [Crgb::new(0, 0, 0); 12];
        Compositor::default().render(&mut composited, &ctx);

        let mut direct = [Crgb::new(0, 0, 0); 12];
        EffectKind::default().render(&mut direct, &ctx);

        assert_eq!(
            composited, direct,
            "segment zero must cover the entire canvas"
        );
        assert!(composited.iter().all(|p| !p.is_black()));
    }

    #[test]
    fn honours_the_configured_effect() {
        let slow = EffectKind::Rainbow(Rainbow::with_step(64));

        let mut a = [Crgb::new(0, 0, 0); 8];
        Compositor::new(slow).render(&mut a, &Ctx::from_millis(1024));

        let mut b = [Crgb::new(0, 0, 0); 8];
        Compositor::default().render(&mut b, &Ctx::from_millis(1024));

        assert_ne!(a, b, "a different effect config must render differently");
    }

    #[test]
    fn empty_canvas_is_a_no_op() {
        Compositor::default().render(&mut [], &Ctx::from_millis(0));
    }
}
