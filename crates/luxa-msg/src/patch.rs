//! Partial updates: what a command changes, field by field.
//!
//! Every field is optional, and `None` means "leave it alone". Values that are
//! relative to the current state — step up, toggle, pick at random — are
//! *described* here and *resolved* by the engine, which alone knows the
//! current value and the field's valid range.

use luxa_color::Rgbw;

use crate::{Name, TransitionTime};

/// An inclusive range that confines a relative [`U8Op`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bounds {
    /// Lowest value.
    pub min: u8,
    /// Highest value.
    pub max: u8,
}

impl Bounds {
    /// The range `min..=max`.
    pub const fn new(min: u8, max: u8) -> Self {
        Self { min, max }
    }
}

/// Which way a [`U8Op::Cycle`] moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Towards larger values.
    Up,
    /// Towards smaller values.
    Down,
}

/// A change to a `u8` field.
///
/// Relative operations work within `bounds` when given, and within the field's
/// own range otherwise — for an effect, the catalogue's effects; for
/// brightness, `0..=255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum U8Op {
    /// Set to this value.
    Set(u8),
    /// Present, but changes nothing.
    ///
    /// Distinct from an absent field because presence alone can matter: naming
    /// an effect, even the current one, stops a running playlist.
    Keep,
    /// Move one step, jumping to the opposite end when the step would leave
    /// the range.
    Cycle {
        /// Which way to step.
        direction: Direction,
        /// The range to cycle through, if narrower than the field's.
        bounds: Option<Bounds>,
    },
    /// Add `delta`, clamped to the range.
    ///
    /// With `wrap`, a value already at the limit it is moving towards jumps to
    /// the opposite end instead of staying put.
    Add {
        /// Amount to add; negative to subtract.
        delta: i16,
        /// Jump to the opposite end when already at the limit.
        wrap: bool,
        /// The range to stay within, if narrower than the field's.
        bounds: Option<Bounds>,
    },
    /// A random value from the range, whose upper bound is exclusive here — so
    /// picking from an effect range `0..count` always yields a valid effect.
    Random {
        /// The range to pick from, if narrower than the field's.
        bounds: Option<Bounds>,
    },
}

/// A change to a `bool` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoolOp {
    /// Set to this value.
    Set(bool),
    /// Invert the current value.
    Toggle,
}

/// A change to one colour slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpec {
    /// Exactly this colour; channels a sender did not give are zero. Black is
    /// `Rgbw(Rgbw::BLACK)`.
    Rgbw(Rgbw),
    /// Only the given channels; the others keep their current value.
    Partial {
        /// Red, if given.
        r: Option<u8>,
        /// Green, if given.
        g: Option<u8>,
        /// Blue, if given.
        b: Option<u8>,
        /// White, if given.
        w: Option<u8>,
    },
    /// White light of this colour temperature, in kelvin. Never zero.
    Kelvin(u16),
    /// A random colour.
    Random,
}

/// Fixture-wide changes.
///
/// The engine applies the fields in a fixed order — brightness before power —
/// so that a patch that both sets a brightness and switches off remembers that
/// brightness for when the fixture comes back on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalPatch {
    /// Master brightness. `0` switches off; anything else switches on.
    pub brightness: Option<U8Op>,
    /// Fixture power.
    pub on: Option<BoolOp>,
    /// New default transition duration.
    pub transition: Option<TransitionTime>,
    /// Transition duration for this change only; the default is unchanged.
    pub transition_once: Option<TransitionTime>,
}

impl GlobalPatch {
    /// A patch that changes nothing.
    pub const NONE: Self = Self {
        brightness: None,
        on: None,
        transition: None,
        transition_once: None,
    };
}

/// Which segments a [`SegmentPatch`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentTarget {
    /// The segment with this id. An id past the last segment, together with a
    /// `stop`, creates one.
    Id(u8),
    /// Every active, selected segment.
    Selected,
}

/// Changes to a segment.
///
/// `NAME` is the byte capacity of a segment name, matching the state's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentPatch<const NAME: usize> {
    /// The segments to change.
    pub target: SegmentTarget,
    /// First LED.
    pub start: Option<u16>,
    /// One past the last LED. `0` deletes the segment.
    pub stop: Option<u16>,
    /// Length; sets `stop` to `start + len` when `stop` is absent.
    pub len: Option<u16>,
    /// New name. An empty name clears it.
    pub name: Option<Name<NAME>>,
    /// Segment power.
    pub on: Option<BoolOp>,
    /// Segment opacity. `0` switches the segment off and keeps its opacity.
    pub opacity: Option<U8Op>,
    /// Primary, background and custom colour; `None` leaves a slot alone.
    pub colors: [Option<ColorSpec>; 3],
    /// Effect.
    pub effect: Option<U8Op>,
    /// Effect speed.
    pub speed: Option<U8Op>,
    /// Effect intensity.
    pub intensity: Option<U8Op>,
    /// Palette.
    pub palette: Option<U8Op>,
    /// Selection.
    pub selected: Option<BoolOp>,
    /// Render back to front.
    pub reverse: Option<BoolOp>,
    /// Mirror around the centre.
    pub mirror: Option<BoolOp>,
}

impl<const NAME: usize> SegmentPatch<NAME> {
    /// A patch for `target` that changes nothing yet.
    pub const fn new(target: SegmentTarget) -> Self {
        Self {
            target,
            start: None,
            stop: None,
            len: None,
            name: None,
            on: None,
            opacity: None,
            colors: [None; 3],
            effect: None,
            speed: None,
            intensity: None,
            palette: None,
            selected: None,
            reverse: None,
            mirror: None,
        }
    }

    /// A patch for the segment with `id`.
    pub const fn for_id(id: u8) -> Self {
        Self::new(SegmentTarget::Id(id))
    }

    /// A patch for every selected segment.
    pub const fn for_selected() -> Self {
        Self::new(SegmentTarget::Selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_patches_change_nothing() {
        assert_eq!(GlobalPatch::default(), GlobalPatch::NONE);

        let seg = SegmentPatch::<8>::for_id(3);
        assert_eq!(seg.target, SegmentTarget::Id(3));
        assert_eq!(seg, SegmentPatch::new(SegmentTarget::Id(3)));
        assert!(seg.colors.iter().all(Option::is_none));
        assert_eq!(
            SegmentPatch::<8>::for_selected().target,
            SegmentTarget::Selected
        );
    }

    /// Every fixture-wide and segment key the first API tier needs has a home.
    #[test]
    fn every_first_tier_key_is_representable() {
        let global = GlobalPatch {
            brightness: Some(U8Op::Add {
                delta: -10,
                wrap: false,
                bounds: None,
            }),
            on: Some(BoolOp::Toggle),
            transition: Some(TransitionTime::from_deciseconds(10)),
            transition_once: Some(TransitionTime::ZERO),
        };
        assert!(global.on.is_some() && global.transition_once.is_some());

        let seg = SegmentPatch::<16> {
            start: Some(0),
            stop: Some(30),
            len: Some(30),
            name: Some(Name::new("Desk")),
            on: Some(BoolOp::Set(true)),
            opacity: Some(U8Op::Set(200)),
            colors: [
                Some(ColorSpec::Rgbw(Rgbw::new(255, 0, 0, 0))),
                Some(ColorSpec::Partial {
                    r: None,
                    g: Some(7),
                    b: None,
                    w: None,
                }),
                Some(ColorSpec::Kelvin(2700)),
            ],
            effect: Some(U8Op::Cycle {
                direction: Direction::Up,
                bounds: None,
            }),
            speed: Some(U8Op::Random {
                bounds: Some(Bounds::new(10, 20)),
            }),
            intensity: Some(U8Op::Keep),
            palette: Some(U8Op::Set(11)),
            selected: Some(BoolOp::Set(false)),
            reverse: Some(BoolOp::Toggle),
            mirror: Some(BoolOp::Set(true)),
            ..SegmentPatch::for_id(1)
        };
        assert_eq!(seg.target, SegmentTarget::Id(1));
        assert_eq!(seg.name.map(|n| n.len()), Some(4));
    }
}
