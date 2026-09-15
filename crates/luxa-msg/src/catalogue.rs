//! Which effect and palette ids exist.

/// A set of `u8` ids.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdSet([u64; 4]);

impl IdSet {
    /// No ids.
    pub const EMPTY: Self = Self([0; 4]);

    /// Every id in `0..end`; `end` is capped at 256.
    pub const fn contiguous(end: u16) -> Self {
        let mut set = Self::EMPTY;
        let mut id = 0;
        while id < end && id < 256 {
            set = set.with(id as u8);
            id += 1;
        }
        set
    }

    /// This set plus `id`.
    #[must_use]
    pub const fn with(self, id: u8) -> Self {
        let mut words = self.0;
        words[(id / 64) as usize] |= 1 << (id % 64);
        Self(words)
    }

    /// Whether `id` is in the set.
    pub const fn contains(self, id: u8) -> bool {
        self.0[(id / 64) as usize] & (1 << (id % 64)) != 0
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0[0] | self.0[1] | self.0[2] | self.0[3] == 0
    }

    /// One past the highest id, or `0` for an empty set.
    ///
    /// Ids below this that are not in the set are *gaps*: numbers reserved
    /// for entries this fixture does not offer.
    pub const fn end(self) -> u16 {
        let mut word = 4;
        while word > 0 {
            word -= 1;
            let bits = self.0[word];
            if bits != 0 {
                return (word as u16) * 64 + (64 - bits.leading_zeros() as u16);
            }
        }
        0
    }
}

/// What choosing an effect with its defaults sets, as the effect's descriptor
/// names them.
///
/// A slider or checkbox that is not named returns to the segment default; the
/// palette, reverse and mirror change only when named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectDefaults {
    /// Effect speed.
    pub speed: Option<u8>,
    /// Effect intensity.
    pub intensity: Option<u8>,
    /// Custom sliders 1–3.
    pub custom: [Option<u8>; 3],
    /// Checkboxes 1–3.
    pub checks: [Option<bool>; 3],
    /// Palette.
    pub palette: Option<u8>,
    /// Render back to front.
    pub reverse: Option<bool>,
    /// Mirror around the centre.
    pub mirror: Option<bool>,
}

impl EffectDefaults {
    /// Nothing named.
    pub const NONE: Self = Self {
        speed: None,
        intensity: None,
        custom: [None; 3],
        checks: [None; 3],
        palette: None,
        reverse: None,
        mirror: None,
    };
}

impl Default for EffectDefaults {
    fn default() -> Self {
        Self::NONE
    }
}

/// The effects and palettes a fixture offers, by id.
///
/// This is all of a catalogue the engine needs: which ids are valid, so it can
/// reject, fall back from and step through effect and palette selections.
/// Names and metadata belong to the API layer.
///
/// Id `0` is always valid for both — it is where an invalid selection falls
/// back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Catalogue {
    /// Valid effect ids.
    pub effects: IdSet,
    /// Valid palette ids.
    pub palettes: IdSet,
    /// The defaults effects name, by effect id. An effect not listed names
    /// none.
    pub defaults: &'static [(u8, EffectDefaults)],
}

impl Catalogue {
    /// A catalogue of the given effect and palette ids, plus id `0` of each.
    pub const fn new(effects: IdSet, palettes: IdSet) -> Self {
        Self {
            effects: effects.with(0),
            palettes: palettes.with(0),
            defaults: &[],
        }
    }

    /// This catalogue with effects naming `defaults`.
    pub const fn with_defaults(mut self, defaults: &'static [(u8, EffectDefaults)]) -> Self {
        self.defaults = defaults;
        self
    }

    /// The defaults `effect` names.
    pub fn defaults_for(&self, effect: u8) -> EffectDefaults {
        self.defaults
            .iter()
            .find(|(id, _)| *id == effect)
            .map_or(EffectDefaults::NONE, |(_, defaults)| *defaults)
    }

    /// Effects `0..effects` and palettes `0..palettes`, with no gaps.
    pub const fn contiguous(effects: u16, palettes: u16) -> Self {
        Self::new(IdSet::contiguous(effects), IdSet::contiguous(palettes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_across_word_boundaries() {
        let set = IdSet::EMPTY.with(0).with(63).with(64).with(255);
        for id in [0, 63, 64, 255] {
            assert!(set.contains(id), "{id}");
        }
        for id in [1, 62, 65, 254] {
            assert!(!set.contains(id), "{id}");
        }
    }

    #[test]
    fn end_is_one_past_the_highest_id() {
        assert_eq!(IdSet::EMPTY.end(), 0);
        assert_eq!(IdSet::EMPTY.with(0).end(), 1);
        assert_eq!(
            IdSet::EMPTY.with(0).with(9).end(),
            10,
            "gaps below still count"
        );
        assert_eq!(IdSet::EMPTY.with(255).end(), 256);
    }

    #[test]
    fn contiguous_sets() {
        assert_eq!(IdSet::contiguous(0), IdSet::EMPTY);
        let set = IdSet::contiguous(220);
        assert_eq!(set.end(), 220);
        assert!(set.contains(219) && !set.contains(220));
        assert_eq!(IdSet::contiguous(1000).end(), 256, "capped");
    }

    #[test]
    fn defaults_are_looked_up_by_effect() {
        const DEFAULTS: &[(u8, EffectDefaults)] = &[(
            9,
            EffectDefaults {
                speed: Some(64),
                ..EffectDefaults::NONE
            },
        )];
        let c = Catalogue::contiguous(10, 1).with_defaults(DEFAULTS);
        assert_eq!(c.defaults_for(9).speed, Some(64));
        assert_eq!(c.defaults_for(3), EffectDefaults::NONE);
        assert_eq!(
            Catalogue::contiguous(10, 1).defaults_for(9),
            EffectDefaults::NONE
        );
    }

    #[test]
    fn id_zero_is_always_in_a_catalogue() {
        let c = Catalogue::new(IdSet::EMPTY.with(9), IdSet::EMPTY);
        assert!(c.effects.contains(0) && c.effects.contains(9));
        assert!(c.palettes.contains(0));
        assert!(!IdSet::EMPTY.is_empty() || IdSet::EMPTY == IdSet::default());
        assert!(!c.effects.is_empty());
    }
}
