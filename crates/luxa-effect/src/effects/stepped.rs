//! Effects from [`smart_leds_fx`], stepped on the animation clock.
//!
//! Those effects advance one step per call and draw over the frame the
//! previous step left. [`Stepped`] decides from the segment's speed when a
//! step is due, and hands the effect the segment's colours and palette, with
//! the segment's sliders and checkboxes mapped onto the named settings the
//! effect reads ([`Controls`]).

use core::fmt;

use luxa_color::{Crgb, Rgbw};
use smart_leds_fx::Setting;
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
    /// The palette would take the background's place, so these effects draw
    /// without one.
    Swapped,
}

/// Which of an effect's named settings a segment's controls drive.
///
/// `smart-leds-fx` names its settings for what they do — a width, a fill, a
/// rate — while a segment carries generic controls whose meaning each effect's
/// descriptor labels. This is the table between the two, one per effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Controls {
    /// The speed slider. `None` keeps its usual job, the time between steps;
    /// a setting instead makes the effect draw every frame.
    pub speed: Option<Setting>,
    /// The intensity slider.
    pub intensity: Option<Setting>,
    /// Custom sliders 1–3.
    pub custom: [Option<Setting>; 3],
    /// Checkboxes 1–3: a setting is on while its checkbox is ticked.
    pub checks: [Option<Setting>; 3],
}

impl Controls {
    /// Speed sets the time between steps, intensity drives the effect's
    /// intensity, and nothing else is mapped.
    pub const STEPPED: Self = Self {
        speed: None,
        intensity: Some(Setting::Intensity),
        custom: [None; 3],
        checks: [None; 3],
    };

    /// With the speed slider driving `setting`.
    pub const fn speed(mut self, setting: Setting) -> Self {
        self.speed = Some(setting);
        self
    }

    /// With the intensity slider driving `setting`.
    pub const fn intensity(mut self, setting: Setting) -> Self {
        self.intensity = Some(setting);
        self
    }

    /// With custom slider `slot` (`0`–`2`) driving `setting`.
    pub const fn custom(mut self, slot: usize, setting: Setting) -> Self {
        self.custom[slot] = Some(setting);
        self
    }

    /// With checkbox `slot` (`0`–`2`) driving `setting`.
    pub const fn check(mut self, slot: usize, setting: Setting) -> Self {
        self.checks[slot] = Some(setting);
        self
    }

    /// Which checkbox switches the effect's overlay drawing on, if any.
    ///
    /// An effect drawing as an overlay leaves everything but its highlights
    /// black, so a renderer can let what lies beneath show through them.
    pub fn overlay_checkbox(&self) -> Option<usize> {
        self.checks
            .iter()
            .position(|setting| *setting == Some(Setting::Overlay))
    }

    fn apply(&self, mut fx: smart_leds_fx::Params, params: &Params) -> smart_leds_fx::Params {
        let sliders = [
            (self.speed, params.speed),
            (self.intensity, params.intensity),
            (self.custom[0], params.custom[0]),
            (self.custom[1], params.custom[1]),
            (self.custom[2], params.custom[2]),
        ];
        for (setting, value) in sliders {
            if let Some(setting) = setting {
                fx = fx.with(setting, value);
            }
        }
        for (setting, ticked) in self.checks.into_iter().zip(params.checks) {
            if let Some(setting) = setting {
                fx = fx.with(setting, u8::from(ticked));
            }
        }
        fx
    }
}

/// A [`smart_leds_fx`] effect, stepped on the animation clock.
///
/// Speed sets the time between steps — [`interval_ms`](Self::interval_ms) —
/// unless its [`Controls`] map it onto a setting, in which case the effect
/// draws every frame. Every control defaults to the middle, where each effect
/// draws its classic look. Between steps the view keeps what the last step
/// drew, so it must be the same buffer every frame.
///
/// Effects that keep only a random seed in their state are seeded from the
/// clock when they start, so two starts differ; the rest start from zero.
#[derive(Clone, Copy)]
pub struct Stepped {
    effect: Fx,
    slots: Slots,
    controls: Controls,
    state: EffectState,
    last_step_ms: Option<u32>,
}

impl Stepped {
    /// `effect`, not yet started.
    pub const fn new(effect: Fx, slots: Slots) -> Self {
        Self {
            effect,
            slots,
            controls: Controls::STEPPED,
            state: EffectState { counter: 0, aux: 0 },
            last_step_ms: None,
        }
    }

    /// With the segment's controls mapped by `controls`.
    pub const fn with_controls(mut self, controls: Controls) -> Self {
        self.controls = controls;
        self
    }

    /// The effect being stepped.
    pub const fn effect(&self) -> Fx {
        self.effect
    }

    /// How a segment's controls reach this effect.
    pub const fn controls(&self) -> Controls {
        self.controls
    }

    /// Milliseconds between steps at `speed`: 10 at full speed, 278 at the
    /// default 128, 1093 at zero. The curve spends most of the slider on the
    /// slower, easier-to-see rates.
    pub const fn interval_ms(speed: u8) -> u32 {
        let slowness = (u8::MAX - speed) as u32;
        10 + slowness * slowness / 60
    }

    fn params(&self, params: &Params, interval_ms: u32, now_ms: u32) -> smart_leds_fx::Params {
        let [primary, background, custom] = params.colors;
        let (first, second) = match self.slots {
            Slots::InOrder => (primary, background),
            Slots::Swapped => (background, primary),
        };
        let mut fx = smart_leds_fx::Params::new([rgb8(first), rgb8(second), rgb8(custom)])
            .speed(interval_ms as u16)
            .now_ms(now_ms);
        if self.slots == Slots::InOrder {
            fx.palette = params.palette;
        }
        self.controls.apply(fx, params)
    }
}

impl Effect for Stepped {
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx, params: &Params) {
        if view.is_empty() {
            return;
        }
        let now = ctx.now_ms();
        let interval = Self::interval_ms(params.speed);

        let (steps, cadence) = match (self.last_step_ms, self.controls.speed) {
            (None, _) => {
                if seeds_from_clock(self.effect) {
                    self.state.aux = (now ^ (view.len() as u32).rotate_left(16)) | 1;
                }
                (1, now)
            }
            // Speed drives a setting, so there is no step interval to wait for.
            (Some(_), Some(_)) => (1, now),
            (Some(last), None) => match now.wrapping_sub(last) / interval {
                0 => return,
                // Behind: take a few steps and restart the cadence from now
                // rather than racing through every missed one.
                due if due > MAX_STEPS_PER_FRAME => (MAX_STEPS_PER_FRAME, now),
                due => (due, last.wrapping_add(due * interval)),
            },
        };

        let fx_params = self.params(params, interval, now);
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
            .field("controls", &self.controls)
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
            && self.controls == other.controls
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
    use luxa_color::CrgbPalette16;

    const RED: Rgbw = Rgbw::new(255, 0, 0, 0);
    const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);
    const BLACK: Crgb = Crgb::new(0, 0, 0);

    fn params(speed: u8) -> Params {
        Params {
            speed,
            intensity: 128,
            colors: [RED, BLUE, Rgbw::BLACK],
            ..Params::DEFAULT
        }
    }

    const GREEN: Crgb = Crgb::new(0, 255, 0);

    fn with_green_palette(speed: u8) -> Params {
        Params {
            palette: Some(CrgbPalette16([GREEN; 16])),
            ..params(speed)
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
    fn a_palette_takes_the_place_of_the_primary_colour() {
        let mut blink = Stepped::new(Fx::Blink, Slots::InOrder);
        let mut view = [BLACK; 3];
        blink.render(&mut view, &Ctx::from_millis(0), &with_green_palette(128));
        assert_eq!(view, [GREEN; 3]);
    }

    #[test]
    fn swapped_slots_draw_without_the_palette() {
        let mut sparkle = Stepped::new(Fx::Sparkle, Slots::Swapped);
        let mut view = [BLACK; 8];
        sparkle.render(&mut view, &Ctx::from_millis(0), &with_green_palette(128));
        assert_eq!(view.iter().filter(|p| **p == BLUE.rgb()).count(), 7);
        assert!(!view.contains(&GREEN));
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
    fn a_speed_mapped_to_a_setting_draws_every_frame() {
        let controls = Controls::STEPPED
            .speed(Setting::Rate)
            .intensity(Setting::Scale);
        let mut sine = Stepped::new(Fx::Sine, Slots::InOrder).with_controls(controls);
        let mut view = [BLACK; 8];
        sine.render(&mut view, &Ctx::from_millis(1_000), &params(128));
        let first = view;
        sine.render(&mut view, &Ctx::from_millis(1_016), &params(128));
        assert_ne!(view, first, "one frame later it has moved");
    }

    #[test]
    fn controls_reach_the_settings_they_are_mapped_to() {
        let percent = |intensity, one_color| {
            let controls = Controls::STEPPED
                .intensity(Setting::Fill)
                .check(0, Setting::OneColor);
            let mut effect = Stepped::new(Fx::Percent, Slots::InOrder).with_controls(controls);
            let p = Params {
                intensity,
                checks: [one_color, false, false],
                ..with_green_palette(255)
            };
            let mut view = [BLACK; 4];
            for frame in 0..8 {
                effect.render(&mut view, &Ctx::from_millis(frame * 10), &p);
            }
            view
        };
        assert_eq!(
            percent(255, false),
            [GREEN; 4],
            "full, following the palette"
        );
        assert_eq!(
            percent(255, true),
            [RED.rgb(); 4],
            "full, in the primary colour"
        );
        assert_eq!(percent(0, false), [BLUE.rgb(); 4], "empty");
    }

    #[test]
    fn an_empty_view_is_a_no_op() {
        let mut scan = Stepped::new(Fx::Scan, Slots::InOrder);
        scan.render(&mut [], &Ctx::from_millis(0), &params(128));
        assert_eq!(scan.last_step_ms, None);
    }
}
