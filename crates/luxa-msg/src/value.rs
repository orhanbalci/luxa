//! Small value types shared by state and commands.

/// An index into the effect catalogue.
///
/// Only an index: which effect it names, and how many exist, is the
/// catalogue's business. `EffectId(0)` is the solid-colour effect by
/// convention.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectId(pub u8);

/// An index into the palette catalogue.
///
/// `PaletteId(0)` is "the effect's default palette" by convention.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaletteId(pub u8);

/// What the LEDs under a segment can physically show.
///
/// A small bit set, reported to clients so they can hide controls that would
/// do nothing — a colour wheel for a white-only strip, a temperature slider
/// for plain RGB.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightCaps(u8);

impl LightCaps {
    /// No light-emitting capability (e.g. a relay).
    pub const NONE: Self = Self(0);
    /// Full-colour RGB.
    pub const RGB: Self = Self(0x01);
    /// A dedicated white channel.
    pub const WHITE: Self = Self(0x02);
    /// Adjustable white colour temperature.
    pub const CCT: Self = Self(0x04);

    const ALL_BITS: u8 = 0x07;

    /// From raw bits; unknown bits are dropped.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    /// The raw bits: `1` RGB, `2` white, `4` CCT.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Both sets of capabilities.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every capability in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A transition duration.
///
/// Held in milliseconds, which is what a renderer needs. Control APIs commonly
/// speak tenths of a second, so both conversions are provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransitionTime {
    ms: u16,
}

impl TransitionTime {
    /// An instant change.
    pub const ZERO: Self = Self::from_millis(0);
    /// The default for a freshly booted fixture: 750 ms.
    pub const DEFAULT: Self = Self::from_millis(750);

    /// A duration of `ms` milliseconds.
    pub const fn from_millis(ms: u16) -> Self {
        Self { ms }
    }

    /// A duration of `ds` tenths of a second, saturating at `u16::MAX` ms.
    pub const fn from_deciseconds(ds: u16) -> Self {
        let ms = ds as u32 * 100;
        Self::from_millis(if ms > u16::MAX as u32 {
            u16::MAX
        } else {
            ms as u16
        })
    }

    /// The duration in milliseconds.
    pub const fn as_millis(self) -> u16 {
        self.ms
    }

    /// The duration in whole tenths of a second, rounded down.
    pub const fn as_deciseconds(self) -> u16 {
        self.ms / 100
    }
}

impl Default for TransitionTime {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A problem worth reporting to clients.
///
/// The numeric codes are part of the control API's wire contract and must not
/// be renumbered. "No error" is `Option::<ErrorCode>::None`, not a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ErrorCode {
    /// Permission denied.
    Denied = 1,
    /// Another client is active.
    Concurrency = 2,
    /// The request buffer was busy; retry later.
    BufferBusy = 3,
    /// The request targets something not implemented.
    NotImplemented = 4,
    /// Not enough RAM for pixel buffers.
    NoRamForPixels = 7,
    /// Effect RAM depleted.
    NoRamForEffects = 8,
    /// The request body could not be parsed.
    Json = 9,
    /// The filesystem could not be initialised.
    FsBegin = 10,
    /// The filesystem is full, or a file size limit was reached.
    FsQuota = 11,
    /// A preset that does not exist was requested.
    FsPresetMissing = 12,
    /// Stored infrared commands were requested but do not exist.
    FsIrCommandsMissing = 13,
    /// Stored remote commands were requested but do not exist.
    FsRemoteCommandsMissing = 14,
    /// An unspecified filesystem error.
    FsGeneral = 19,
    /// A temperature sensor is above its threshold.
    OverTemperature = 30,
    /// A current sensor is above its threshold.
    OverCurrent = 31,
    /// A voltage sensor is below its threshold.
    UnderVoltage = 32,
    /// Low on RAM.
    LowMemory = 33,
    /// Low on effect data memory.
    LowSegmentMemory = 34,
    /// Low on WebSocket memory.
    LowWebSocketMemory = 35,
    /// Low on pixel buffer memory.
    LowPixelBufferMemory = 37,
    /// Rebooted after an error; rolling back.
    RebootedAfterError = 90,
    /// Rebooted after a brownout.
    RebootedAfterBrownout = 91,
    /// A hardware setting changed and needs a reboot.
    RebootNeeded = 100,
    /// A hardware setting changed and needs a power cycle.
    PowerCycleNeeded = 101,
}

impl ErrorCode {
    /// Every variant, for exhaustive checks.
    pub const ALL: [Self; 24] = [
        Self::Denied,
        Self::Concurrency,
        Self::BufferBusy,
        Self::NotImplemented,
        Self::NoRamForPixels,
        Self::NoRamForEffects,
        Self::Json,
        Self::FsBegin,
        Self::FsQuota,
        Self::FsPresetMissing,
        Self::FsIrCommandsMissing,
        Self::FsRemoteCommandsMissing,
        Self::FsGeneral,
        Self::OverTemperature,
        Self::OverCurrent,
        Self::UnderVoltage,
        Self::LowMemory,
        Self::LowSegmentMemory,
        Self::LowWebSocketMemory,
        Self::LowPixelBufferMemory,
        Self::RebootedAfterError,
        Self::RebootedAfterBrownout,
        Self::RebootNeeded,
        Self::PowerCycleNeeded,
    ];

    /// The wire code.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The variant for a wire code, if it is one. `0` ("no error") is `None`.
    pub const fn from_code(code: u8) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as u8 == code {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

/// A command's position in the engine's input order.
///
/// Producers number commands as they enqueue them; the engine records the
/// highest it has applied in [`State::applied_seq`](crate::State::applied_seq).
/// A sender waiting for the result of command `n` waits for a published state
/// with `applied_seq >= n`.
///
/// `Seq(0)` means "nothing applied yet", so the first command is `Seq(1)`.
/// The counter is `u32`: at one command per millisecond it lasts 49 days before
/// wrapping, and wrapping only risks waking one waiter early.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Seq(pub u32);

impl Seq {
    /// Nothing applied yet.
    pub const ZERO: Self = Self(0);

    /// The number after this one, wrapping past `u32::MAX` to `1`.
    #[must_use]
    pub const fn next(self) -> Self {
        match self.0.checked_add(1) {
            Some(n) => Self(n),
            None => Self(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_caps_bits_match_the_wire_values() {
        assert_eq!(LightCaps::RGB.bits(), 1);
        assert_eq!(LightCaps::WHITE.bits(), 2);
        assert_eq!(LightCaps::CCT.bits(), 4);
        assert_eq!(
            LightCaps::from_bits(0xFF).bits(),
            7,
            "unknown bits are dropped"
        );
    }

    #[test]
    fn light_caps_union_and_contains() {
        let rgbw = LightCaps::RGB.union(LightCaps::WHITE);
        assert!(rgbw.contains(LightCaps::RGB));
        assert!(rgbw.contains(LightCaps::WHITE));
        assert!(!rgbw.contains(LightCaps::CCT));
        assert!(rgbw.contains(LightCaps::NONE));
    }

    #[test]
    fn transition_converts_between_units() {
        assert_eq!(TransitionTime::DEFAULT.as_millis(), 750);
        assert_eq!(TransitionTime::DEFAULT.as_deciseconds(), 7, "rounded down");
        assert_eq!(TransitionTime::from_deciseconds(12).as_millis(), 1200);
        assert_eq!(TransitionTime::default(), TransitionTime::DEFAULT);
    }

    #[test]
    fn transition_saturates_instead_of_wrapping() {
        assert_eq!(
            TransitionTime::from_deciseconds(u16::MAX).as_millis(),
            u16::MAX
        );
    }

    #[test]
    fn error_codes_round_trip() {
        for e in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_code(e.code()), Some(e));
        }
        assert_eq!(ErrorCode::from_code(0), None, "0 means no error");
        assert_eq!(ErrorCode::from_code(5), None);
    }

    #[test]
    fn error_codes_are_unique() {
        for (i, a) in ErrorCode::ALL.iter().enumerate() {
            for b in &ErrorCode::ALL[i + 1..] {
                assert_ne!(a.code(), b.code());
            }
        }
    }

    #[test]
    fn seq_starts_after_zero_and_skips_it_on_wrap() {
        assert_eq!(Seq::ZERO.next(), Seq(1));
        assert_eq!(Seq(u32::MAX).next(), Seq(1));
    }
}
