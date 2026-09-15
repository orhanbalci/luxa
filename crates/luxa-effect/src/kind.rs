//! The effect registry.

use luxa_color::Crgb;

use crate::effects::{Rainbow, Solid};
use crate::{Ctx, Descriptor, Effect, Params};

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
}

impl EffectKind {
    /// Every effect the registry knows, in id order.
    pub const ALL: &'static [EffectKind] = &[
        EffectKind::Solid(Solid::new()),
        EffectKind::Rainbow(Rainbow::new()),
    ];

    /// The effect's id: a stable number, part of the control API's wire
    /// contract.
    ///
    /// Numbering follows existing LED controllers, so their clients select
    /// the effect they mean. Ids this registry has no effect for are gaps, and
    /// id `0` — solid colour — always exists.
    pub const fn id(&self) -> u8 {
        match self {
            Self::Solid(_) => 0,
            Self::Rainbow(_) => 9,
        }
    }

    /// The effect's descriptor: its name and the controls a UI shows for it.
    pub const fn descriptor(&self) -> Descriptor<'static> {
        Descriptor::new(match self {
            Self::Solid(_) => "Solid",
            Self::Rainbow(_) => "Rainbow@!,Size;;!",
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn every_effect_is_found_by_its_id() {
        for effect in EffectKind::ALL {
            assert_eq!(EffectKind::from_id(effect.id()), Some(*effect));
        }
        assert_eq!(EffectKind::from_id(1), None, "a gap");
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
    }
}
