//! The effect registry.

use luxa_color::Crgb;
use smart_leds_fx::effect::Effect as Fx;

use crate::effects::{Rainbow, Slots, Solid, Stepped};
use crate::{Ctx, Descriptor, Effect, Params};

const SOLID_ID: u8 = 0;
const RAINBOW_ID: u8 = 9;

/// A [`smart_leds_fx`] effect, filed under the id and name clients know.
struct Listing {
    id: u8,
    descriptor: &'static str,
    effect: Fx,
    slots: Slots,
}

const fn listed(id: u8, descriptor: &'static str, effect: Fx) -> Listing {
    Listing {
        id,
        descriptor,
        effect,
        slots: Slots::InOrder,
    }
}

const fn swapped(id: u8, descriptor: &'static str, effect: Fx) -> Listing {
    Listing {
        slots: Slots::Swapped,
        ..listed(id, descriptor, effect)
    }
}

/// The [`smart_leds_fx`] effects, in id order, each under the id and name
/// existing LED controller clients use for the closest effect they know.
///
/// Descriptors show only the controls the effect reads: speed always,
/// intensity where it shapes the effect, the colour slots it draws with, and
/// the palette, which takes the place of the primary colour. Effects with
/// swapped slots would draw the palette behind, so they draw without one.
const STEPPED: [Listing; 41] = [
    listed(1, "Blink@!;!,!;!;", Fx::Blink),
    listed(2, "Breathe@!;!;!;", Fx::Breath),
    listed(3, "Wipe@!;!,!;!;", Fx::ColorWipe),
    listed(4, "Wipe Random@!;;!;", Fx::ColorWipeRandom),
    listed(5, "Random Colors@!;;!;", Fx::RandomColor),
    listed(7, "Dynamic@!;;!;", Fx::MultiDynamic),
    listed(8, "Colorloop@!;;!;", Fx::Rainbow),
    listed(10, "Scan@!;!,!;!;", Fx::Scan),
    listed(11, "Scan Dual@!;!,!;!;", Fx::DualScan),
    listed(12, "Fade@!;!,!;!;", Fx::Fade),
    listed(13, "Theater@!;!,!;!;", Fx::TheaterChase),
    listed(14, "Theater Rainbow@!;,!;!;", Fx::TheaterChaseRainbow),
    listed(15, "Running@!;!,!;!;", Fx::RunningLights),
    listed(17, "Twinkle@!;!,!;!;", Fx::Twinkle),
    listed(18, "Dissolve@!;!,!,!;!;", Fx::BlockDissolve),
    swapped(20, "Sparkle@!;!,!;;", Fx::Sparkle),
    listed(21, "Sparkle Dark@!;!,!;!;", Fx::Sparkle),
    listed(22, "Sparkle+@!,!;!;!;", Fx::HyperSparkle),
    listed(23, "Strobe@!,!;!,!;!;", Fx::Strobe),
    listed(24, "Strobe Rainbow@!,!;,!;!;", Fx::StrobeRainbow),
    listed(25, "Strobe Mega@!;!,!;!;", Fx::MultiStrobe),
    listed(26, "Blink Rainbow@!;,!;!;", Fx::BlinkRainbow),
    listed(28, "Chase@!;!,!,!;!;", Fx::BicolorChase),
    listed(29, "Chase Random@!;;!;", Fx::ChaseRandom),
    listed(30, "Chase Rainbow@!;;!;", Fx::ChaseRainbow),
    listed(31, "Chase Flash@!;!;!;", Fx::ChaseFlash),
    listed(32, "Chase Flash Rnd@!;!;!;", Fx::ChaseFlashRandom),
    listed(33, "Rainbow Runner@!;;!;", Fx::ChaseRainbowWhite),
    listed(36, "Sweep Random@!;;!;", Fx::ColorSweepRandom),
    listed(40, "Scanner@!,!;!;!;", Fx::LarsonScanner),
    listed(41, "Lighthouse@!,!;!;!;", Fx::Comet),
    listed(42, "Fireworks@!,!;!;!;", Fx::Fireworks),
    listed(43, "Rain@!,!;!,,!;!;", Fx::Rain),
    listed(45, "Fire Flicker@!,!;!;!;", Fx::FireFlicker),
    listed(54, "Chase 3@!;!,!,!;!;", Fx::TricolorChase),
    listed(56, "Tri Fade@!;!,!,!;!;", Fx::TriFade),
    listed(58, "ICU@!;!;!;", Fx::Icu),
    listed(59, "Multi Comet@!,!;!,,!;!;", Fx::MultiComet),
    listed(60, "Scanner Dual@!,!;!,,!;!;", Fx::DualLarson),
    listed(80, "Twinklefox@!;!,!,!;!;", Fx::TwinkleFox),
    listed(100, "Heartbeat@!,!;!;!;", Fx::Heartbeat),
];

const COUNT: usize = STEPPED.len() + 2;

/// Every effect, in id order: the native ones merged into [`STEPPED`].
const REGISTRY: [EffectKind; COUNT] = {
    let mut all = [EffectKind::Solid(Solid::new()); COUNT];
    let mut next = 1;
    let mut rainbow_placed = false;
    let mut i = 0;
    while i < STEPPED.len() {
        if !rainbow_placed && STEPPED[i].id > RAINBOW_ID {
            all[next] = EffectKind::Rainbow(Rainbow::new());
            next += 1;
            rainbow_placed = true;
        }
        all[next] = EffectKind::Stepped {
            id: STEPPED[i].id,
            effect: Stepped::new(STEPPED[i].effect, STEPPED[i].slots),
        };
        next += 1;
        i += 1;
    }
    if !rainbow_placed {
        all[next] = EffectKind::Rainbow(Rainbow::new());
    }
    all
};

/// Every effect Luxa can run, as one statically-dispatched value.
///
/// This is the registry, and its shape is the point. An `EffectKind` is a
/// plain enum sized to its largest variant: storing one costs no allocation,
/// rendering one is a `match` the optimizer can see through, and the whole
/// render path stays `no_std` with no vtables. A `Box<dyn Effect>` would be
/// none of those things on a device with no allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// See [`Solid`].
    Solid(Solid),
    /// See [`Rainbow`].
    Rainbow(Rainbow),
    /// A [`smart_leds_fx`] effect; see [`Stepped`].
    Stepped {
        /// The id it is listed under.
        id: u8,
        /// The running effect.
        effect: Stepped,
    },
}

impl EffectKind {
    /// Every effect the registry knows, in id order.
    pub const ALL: &'static [EffectKind] = &REGISTRY;

    /// The effect's id: a stable number, part of the control API's wire
    /// contract.
    ///
    /// Numbering follows existing LED controllers, so their clients select
    /// the effect they mean. Ids this registry has no effect for are gaps, and
    /// id `0` — solid colour — always exists.
    pub const fn id(&self) -> u8 {
        match self {
            Self::Solid(_) => SOLID_ID,
            Self::Rainbow(_) => RAINBOW_ID,
            Self::Stepped { id, .. } => *id,
        }
    }

    /// The effect's descriptor: its name and the controls a UI shows for it.
    pub const fn descriptor(&self) -> Descriptor<'static> {
        Descriptor::new(match self {
            Self::Solid(_) => "Solid",
            Self::Rainbow(_) => "Rainbow@!,Size;;!",
            Self::Stepped { id, .. } => {
                let mut i = 0;
                loop {
                    if STEPPED[i].id == *id {
                        break STEPPED[i].descriptor;
                    }
                    i += 1;
                }
            }
        })
    }

    /// The effect's display name.
    pub fn name(&self) -> &'static str {
        self.descriptor().name()
    }

    /// A fresh instance of the effect with `id`, if the registry has one.
    pub const fn from_id(id: u8) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i].id() == id {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

impl Default for EffectKind {
    /// Solid colour, effect `0`.
    fn default() -> Self {
        Self::Solid(Solid::new())
    }
}

impl Effect for EffectKind {
    /// Dispatches to the active variant. No `dyn`, no vtable.
    #[inline]
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx, params: &Params) {
        match self {
            Self::Solid(e) => e.render(view, ctx, params),
            Self::Rainbow(e) => e.render(view, ctx, params),
            Self::Stepped { effect, .. } => effect.render(view, ctx, params),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_color::{LAVA_COLORS, Rgbw};

    #[test]
    fn dispatch_matches_the_concrete_effect() {
        let ctx = Ctx::from_millis(777);
        let params = Params::DEFAULT;

        let mut direct = [Crgb::new(0, 0, 0); 8];
        Rainbow::new().render(&mut direct, &ctx, &params);

        let mut through_enum = [Crgb::new(0, 0, 0); 8];
        EffectKind::Rainbow(Rainbow::new()).render(&mut through_enum, &ctx, &params);

        assert_eq!(direct, through_enum);
    }

    #[test]
    fn ids_are_unique_and_in_order() {
        for pair in EffectKind::ALL.windows(2) {
            assert!(pair[0].id() < pair[1].id(), "{:?}", pair);
        }
        assert_eq!(EffectKind::ALL.len(), STEPPED.len() + 2);
    }

    #[test]
    fn every_effect_is_found_by_its_id() {
        for effect in EffectKind::ALL {
            assert_eq!(EffectKind::from_id(effect.id()), Some(*effect));
        }
        assert_eq!(EffectKind::from_id(6), None, "a gap");
        assert_eq!(EffectKind::from_id(255), None);
    }

    #[test]
    fn solid_colour_is_effect_zero_and_the_default() {
        assert_eq!(EffectKind::default().id(), 0);
        assert_eq!(EffectKind::ALL[0].id(), 0);
    }

    #[test]
    fn descriptors_name_the_effects() {
        assert_eq!(EffectKind::Solid(Solid::new()).name(), "Solid");
        let rainbow = EffectKind::Rainbow(Rainbow::new());
        assert_eq!(rainbow.name(), "Rainbow");
        assert!(rainbow.descriptor().palette().is_shown());

        let scanner = EffectKind::from_id(40).unwrap();
        assert_eq!(scanner.name(), "Scanner");
        assert!(scanner.descriptor().slider(1).is_shown(), "intensity");
        assert!(!scanner.descriptor().color(1).is_shown(), "no background");
    }

    #[test]
    fn stepped_effects_show_speed_and_the_palette_when_they_draw_with_it() {
        for listing in &STEPPED {
            let effect = EffectKind::from_id(listing.id).unwrap();
            let descriptor = effect.descriptor();
            assert!(!effect.name().is_empty());
            assert!(descriptor.slider(0).is_shown(), "{}", effect.name());
            assert_eq!(
                descriptor.palette().is_shown(),
                listing.slots == Slots::InOrder,
                "{}",
                effect.name()
            );
        }
    }

    #[test]
    fn every_effect_renders_any_length_without_panicking() {
        for palette in [None, Some(LAVA_COLORS)] {
            let params = Params {
                speed: 255,
                intensity: 200,
                colors: [
                    Rgbw::new(255, 80, 0, 0),
                    Rgbw::new(0, 40, 200, 0),
                    Rgbw::new(30, 220, 90, 0),
                ],
                palette,
            };
            for effect in EffectKind::ALL {
                for len in [0, 1, 2, 7, 60] {
                    let mut instance = *effect;
                    let mut view = [Crgb::new(0, 0, 0); 60];
                    for frame in 0..100 {
                        instance.render(&mut view[..len], &Ctx::from_millis(frame * 16), &params);
                    }
                }
            }
        }
    }
}
