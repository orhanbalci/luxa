//! Effects from [`smart_leds_fx`], stepped on the animation clock.
//!
//! Those effects advance one step per call and draw over the frame the
//! previous step left. [`Stepped`] decides from the segment's speed when a
//! step is due, and hands the effect the segment's colours and intensity.

use core::fmt;

use luxa_color::{Crgb, Rgbw};
use smart_leds_fx::effect::Effect as Fx;
use smart_leds_fx::prelude::RGB8;
use smart_leds_fx::segment::EffectState;
use smart_leds_fx::utils::rgb;

use crate::{Ctx, Effect, Params};

/// Most steps one frame takes when steps fall due faster than frames, or when
/// rendering fell behind.
const MAX_STEPS_PER_FRAME: u32 = 4;

/// How a segment's colour slots reach the effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slots {
    /// Primary, background and custom, in that order.
    InOrder,
    /// Primary and background exchanged: for an effect that draws its first
    /// colour behind its second, where clients expect the primary in front.
    Swapped,
}

/// A [`smart_leds_fx`] effect, stepped on the animation clock.
///
/// Speed sets the time between steps — [`interval_ms`](Self::interval_ms) —
/// and intensity passes straight through; both default to the middle, where
/// each effect draws its classic look. Between steps the view keeps what the
/// last step drew, so it must be the same buffer every frame.
///
/// Effects that keep only a random seed in their state are seeded from the
/// clock when they start, so two starts differ; the rest start from zero.
#[derive(Clone, Copy)]
pub struct Stepped {
    effect: Fx,
    slots: Slots,
    state: EffectState,
    last_step_ms: Option<u32>,
}

impl Stepped {
    /// `effect`, not yet started.
    pub const fn new(effect: Fx, slots: Slots) -> Self {
        Self {
            effect,
            slots,
            state: EffectState { counter: 0, aux: 0 },
            last_step_ms: None,
        }
    }

    /// The effect being stepped.
    pub const fn effect(&self) -> Fx {
        self.effect
    }

    /// Milliseconds between steps at `speed`: 10 at full speed, 278 at the
    /// default 128, 1093 at zero. The curve spends most of the slider on the
    /// slower, easier-to-see rates.
    pub const fn interval_ms(speed: u8) -> u32 {
        let slowness = (u8::MAX - speed) as u32;
        10 + slowness * slowness / 60
    }

    fn params(&self, params: &Params, interval_ms: u32) -> smart_leds_fx::Params {
        let [primary, background, custom] = params.colors;
        let (first, second) = match self.slots {
            Slots::InOrder => (primary, background),
            Slots::Swapped => (background, primary),
        };
        smart_leds_fx::Params::new([rgb8(first), rgb8(second), rgb8(custom)])
            .speed(interval_ms as u16)
            .intensity(params.intensity)
    }
}

impl Effect for Stepped {
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx, params: &Params) {
        if view.is_empty() {
            return;
        }
        let now = ctx.now_ms();
        let interval = Self::interval_ms(params.speed);

        let (steps, cadence) = match self.last_step_ms {
            None => {
                if seeds_from_clock(self.effect) {
                    self.state.aux = (now ^ (view.len() as u32).rotate_left(16)) | 1;
                }
                (1, now)
            }
            Some(last) => match now.wrapping_sub(last) / interval {
                0 => return,
                // Behind: take a few steps and restart the cadence from now
                // rather than racing through every missed one.
                due if due > MAX_STEPS_PER_FRAME => (MAX_STEPS_PER_FRAME, now),
                due => (due, last.wrapping_add(due * interval)),
            },
        };

        let fx_params = self.params(params, interval);
        for _ in 0..steps {
            self.effect.step(view, &mut self.state, &fx_params);
        }
        self.last_step_ms = Some(cadence);
    }
}

impl fmt::Debug for Stepped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stepped")
            .field("effect", &self.effect)
            .field("slots", &self.slots)
            .field("counter", &self.state.counter)
            .field("aux", &self.state.aux)
            .field("last_step_ms", &self.last_step_ms)
            .finish()
    }
}

impl PartialEq for Stepped {
    fn eq(&self, other: &Self) -> bool {
        self.effect == other.effect
            && self.slots == other.slots
            && self.state.counter == other.state.counter
            && self.state.aux == other.state.aux
            && self.last_step_ms == other.last_step_ms
    }
}

// Every field compares totally: the effect's variants carry no data.
impl Eq for Stepped {}

fn rgb8(color: Rgbw) -> RGB8 {
    let c = color.rgb();
    rgb(c.r, c.g, c.b)
}

/// Whether the effect keeps nothing but a random seed in `state.aux`, so a
/// seed from the clock varies its pattern without confusing it. The others
/// pack directions, positions or a first-frame flag there and must start from
/// zero.
const fn seeds_from_clock(effect: Fx) -> bool {
    matches!(
        effect,
        Fx::RandomColor
            | Fx::SingleDynamic
            | Fx::MultiDynamic
            | Fx::BlockDissolve
            | Fx::Twinkle
            | Fx::HyperSparkle
            | Fx::Fireworks
            | Fx::Rain
            | Fx::FireFlicker
            | Fx::MultiComet
            | Fx::ColorWipeRandom
            | Fx::ColorSweepRandom
            | Fx::ChaseRandom
            | Fx::ChaseFlashRandom
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgbw = Rgbw::new(255, 0, 0, 0);
    const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);
    const BLACK: Crgb = Crgb::new(0, 0, 0);

    fn params(speed: u8) -> Params {
        Params {
            speed,
            intensity: 128,
            colors: [RED, BLUE, Rgbw::BLACK],
        }
    }

    #[test]
    fn speed_sets_the_time_between_steps() {
        assert_eq!(Stepped::interval_ms(255), 10);
        assert_eq!(Stepped::interval_ms(128), 278);
        assert_eq!(Stepped::interval_ms(0), 1093);
    }

    #[test]
    fn steps_only_when_one_is_due() {
        let mut blink = Stepped::new(Fx::Blink, Slots::InOrder);
        let p = params(128);
        let mut view = [BLACK; 3];

        blink.render(&mut view, &Ctx::from_millis(1_000), &p);
        assert_eq!(view, [RED.rgb(); 3], "the first frame steps at once");

        blink.render(&mut view, &Ctx::from_millis(1_200), &p);
        assert_eq!(view, [RED.rgb(); 3], "not due yet");

        blink.render(&mut view, &Ctx::from_millis(1_278), &p);
        assert_eq!(
            view,
            [BLUE.rgb(); 3],
            "the second step shows the background"
        );
    }

    #[test]
    fn a_frame_far_behind_takes_a_few_steps_not_all() {
        let mut blink = Stepped::new(Fx::Blink, Slots::InOrder);
        let p = params(128);
        let mut view = [BLACK; 3];
        blink.render(&mut view, &Ctx::from_millis(0), &p);
        blink.render(&mut view, &Ctx::from_millis(60_000), &p);
        assert_eq!(blink.state.counter, 1 + MAX_STEPS_PER_FRAME);
        assert_eq!(blink.last_step_ms, Some(60_000));
    }

    #[test]
    fn steps_keep_their_cadence_when_frames_arrive_late() {
        let mut blink = Stepped::new(Fx::Blink, Slots::InOrder);
        let p = params(128);
        let mut view = [BLACK; 3];
        blink.render(&mut view, &Ctx::from_millis(0), &p);
        blink.render(&mut view, &Ctx::from_millis(290), &p);
        assert_eq!(blink.last_step_ms, Some(278), "the next step is due at 556");
    }

    #[test]
    fn swapped_slots_put_the_primary_colour_in_front() {
        let count = |slots, colour: Crgb| {
            let mut sparkle = Stepped::new(Fx::Sparkle, slots);
            let mut view = [BLACK; 8];
            sparkle.render(&mut view, &Ctx::from_millis(0), &params(128));
            view.iter().filter(|p| **p == colour).count()
        };
        assert_eq!(count(Slots::InOrder, RED.rgb()), 7, "primary behind");
        assert_eq!(count(Slots::Swapped, BLUE.rgb()), 7, "background behind");
    }

    #[test]
    fn random_effects_start_differently_and_the_rest_start_the_same() {
        let first_frame = |effect, now_ms| {
            let mut stepped = Stepped::new(effect, Slots::InOrder);
            let mut view = [BLACK; 16];
            stepped.render(&mut view, &Ctx::from_millis(now_ms), &params(128));
            view
        };
        assert_ne!(
            first_frame(Fx::MultiDynamic, 1_000),
            first_frame(Fx::MultiDynamic, 2_000)
        );
        assert_eq!(first_frame(Fx::Scan, 1_000), first_frame(Fx::Scan, 2_000));
    }

    #[test]
    fn an_empty_view_is_a_no_op() {
        let mut scan = Stepped::new(Fx::Scan, Slots::InOrder);
        scan.render(&mut [], &Ctx::from_millis(0), &params(128));
        assert_eq!(scan.last_step_ms, None);
    }
}
