//! The effect registry.

use luxa_color::Crgb;

use crate::effects::Rainbow;
use crate::{Ctx, Effect};

/// Every effect Luxa can run, as one statically-dispatched value.
///
/// This is the registry, and its shape is the point. An `EffectKind` is a
/// plain enum sized to its largest variant: storing one costs no allocation,
/// rendering one is a `match` the optimizer can see through, and the whole
/// render path stays `no_std` with no vtables. A `Box<dyn Effect>` would be
/// none of those things on a device with no allocator.
///
/// Slice 1 has one variant, which makes the `match` trivial — deliberately so.
/// Establishing the pattern while it is trivial means the slice that adds
/// forty effects is forty variants and no new architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// See [`Rainbow`].
    Rainbow(Rainbow),
}

impl EffectKind {
    /// A short, stable identifier — for the UI, config and logs.
    ///
    /// Stable across releases: these strings are part of the wire contract
    /// once effect selection exists.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::Rainbow(_) => "rainbow",
        }
    }

    /// Every effect the registry knows, in UI order.
    pub const ALL: &'static [EffectKind] = &[EffectKind::Rainbow(Rainbow::new())];
}

impl Default for EffectKind {
    fn default() -> Self {
        Self::Rainbow(Rainbow::new())
    }
}

impl Effect for EffectKind {
    /// Dispatches to the active variant. No `dyn`, no vtable.
    #[inline]
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx) {
        match self {
            Self::Rainbow(e) => e.render(view, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_matches_the_concrete_effect() {
        let ctx = Ctx::from_millis(777);

        let mut direct = [Crgb::new(0, 0, 0); 8];
        Rainbow::new().render(&mut direct, &ctx);

        let mut through_enum = [Crgb::new(0, 0, 0); 8];
        EffectKind::Rainbow(Rainbow::new()).render(&mut through_enum, &ctx);

        assert_eq!(direct, through_enum);
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in EffectKind::ALL.iter().enumerate() {
            for b in &EffectKind::ALL[i + 1..] {
                assert_ne!(a.id(), b.id(), "duplicate effect id {}", a.id());
            }
        }
    }

    #[test]
    fn default_is_the_slice_one_effect() {
        assert_eq!(EffectKind::default().id(), "rainbow");
    }
}
