//! Fixture state: what the engine owns and publishes.

use core::fmt;

use luxa_color::Rgbw;

use crate::{EffectId, Name, PaletteId, Seq, TransitionTime};

/// The fixture a [`State`] describes.
///
/// Passed in when state is created rather than baked in as a constant, so the
/// same types serve any strip. Per-range light capabilities join this later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layout {
    /// Total number of addressable LEDs.
    pub led_count: u16,
}

impl Layout {
    /// A fixture of `led_count` LEDs.
    pub const fn new(led_count: u16) -> Self {
        Self { led_count }
    }
}

/// A contiguous range of LEDs running one effect with its own settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Segment<const NAME: usize> {
    /// First LED, inclusive.
    pub start: u16,
    /// Last LED, exclusive. Always greater than `start`.
    pub stop: u16,
    /// Segment power, independent of the fixture's.
    pub on: bool,
    /// Segment opacity, `0`–`255`.
    pub opacity: u8,
    /// Primary, background and custom colours.
    pub colors: [Rgbw; 3],
    /// The effect this segment runs.
    pub effect: EffectId,
    /// Effect speed, `0`–`255`.
    pub speed: u8,
    /// Effect intensity, `0`–`255`.
    pub intensity: u8,
    /// The palette the effect draws from.
    pub palette: PaletteId,
    /// Render the effect back to front.
    pub reverse: bool,
    /// Mirror the effect around the segment's centre.
    pub mirror: bool,
    /// Selected for commands that target "the selected segments".
    pub selected: bool,
    /// Display name; empty when unnamed.
    pub name: Name<NAME>,
}

impl<const NAME: usize> Segment<NAME> {
    /// Default effect speed.
    pub const DEFAULT_SPEED: u8 = 128;
    /// Default effect intensity.
    pub const DEFAULT_INTENSITY: u8 = 128;
    /// Warm orange: the primary colour of automatically created segments, so a
    /// freshly booted fixture visibly lights up.
    pub const DEFAULT_COLOR: Rgbw = Rgbw::from_u32(0xFFA000);

    /// A plain segment over `[start, stop)`: on, fully opaque, selected, all
    /// colours black, effect 0 at default speed and intensity, unnamed.
    ///
    /// A `stop` not after `start` yields a one-LED segment — a segment is never
    /// empty.
    pub const fn new(start: u16, stop: u16) -> Self {
        Self {
            start,
            stop: if stop > start {
                stop
            } else {
                start.saturating_add(1)
            },
            on: true,
            opacity: u8::MAX,
            colors: [Rgbw::BLACK; 3],
            effect: EffectId(0),
            speed: Self::DEFAULT_SPEED,
            intensity: Self::DEFAULT_INTENSITY,
            palette: PaletteId(0),
            reverse: false,
            mirror: false,
            selected: true,
            name: Name::EMPTY,
        }
    }

    /// A segment as created automatically for a strip: like [`new`](Self::new),
    /// with the primary colour set to [`DEFAULT_COLOR`](Self::DEFAULT_COLOR).
    pub const fn auto(start: u16, stop: u16) -> Self {
        let mut segment = Self::new(start, stop);
        segment.colors[0] = Self::DEFAULT_COLOR;
        segment
    }

    /// Number of LEDs covered.
    pub const fn len(&self) -> u16 {
        self.stop - self.start
    }

    /// Always `false`: a segment covers at least one LED.
    pub const fn is_empty(&self) -> bool {
        self.stop == self.start
    }
}

/// Everything the fixture currently is.
///
/// The engine owns one `State` and is its only writer; everyone else reads
/// published copies. Capacity is chosen by the application: `SEGMENTS` is the
/// most segments it can hold, `NAME` the byte capacity of a segment name.
#[derive(Clone)]
pub struct State<const SEGMENTS: usize, const NAME: usize> {
    /// Master brightness. `0` means the fixture is off.
    pub brightness: u8,
    /// The brightness to return to when switched back on. Never `0` in a
    /// state the engine produced.
    pub last_brightness: u8,
    /// Duration of transitions between states.
    pub transition: TransitionTime,
    /// Index of the main segment, which single-segment controls act on.
    pub main_segment: u8,
    /// The highest command sequence number the engine has applied.
    pub applied_seq: Seq,
    /// Slots past `segment_count` are unused and never observable.
    segments: [Segment<NAME>; SEGMENTS],
    segment_count: usize,
}

impl<const SEGMENTS: usize, const NAME: usize> State<SEGMENTS, NAME> {
    /// Brightness of a freshly booted fixture.
    pub const DEFAULT_BRIGHTNESS: u8 = 128;

    /// A freshly booted fixture: on at brightness 128, 750 ms transitions, and
    /// one automatic segment covering every LED.
    pub fn new(layout: Layout) -> Self {
        const {
            assert!(
                SEGMENTS > 0,
                "State needs capacity for at least one segment"
            );
            assert!(
                NAME <= u8::MAX as usize,
                "segment names are limited to 255 bytes"
            );
        };

        let mut segments = [Segment::new(0, 1); SEGMENTS];
        segments[0] = Segment::auto(0, layout.led_count);
        Self {
            brightness: Self::DEFAULT_BRIGHTNESS,
            last_brightness: Self::DEFAULT_BRIGHTNESS,
            transition: TransitionTime::DEFAULT,
            main_segment: 0,
            applied_seq: Seq::ZERO,
            segments,
            segment_count: 1,
        }
    }

    /// Whether the fixture is on.
    pub const fn is_on(&self) -> bool {
        self.brightness > 0
    }

    /// Whether the command numbered `seq` is reflected in this state.
    pub fn has_applied(&self, seq: Seq) -> bool {
        self.applied_seq >= seq
    }

    /// The active segments, in order.
    pub fn segments(&self) -> &[Segment<NAME>] {
        &self.segments[..self.segment_count]
    }

    /// The active segments, mutably.
    pub fn segments_mut(&mut self) -> &mut [Segment<NAME>] {
        &mut self.segments[..self.segment_count]
    }

    /// Appends a segment, returning its index — or the segment back if the
    /// state is at capacity.
    pub fn push_segment(&mut self, segment: Segment<NAME>) -> Result<usize, Segment<NAME>> {
        if self.segment_count == SEGMENTS {
            return Err(segment);
        }
        self.segments[self.segment_count] = segment;
        self.segment_count += 1;
        Ok(self.segment_count - 1)
    }

    /// The most segments this state can hold.
    pub const fn segment_capacity(&self) -> usize {
        SEGMENTS
    }
}

impl<const SEGMENTS: usize, const NAME: usize> PartialEq for State<SEGMENTS, NAME> {
    fn eq(&self, other: &Self) -> bool {
        self.brightness == other.brightness
            && self.last_brightness == other.last_brightness
            && self.transition == other.transition
            && self.main_segment == other.main_segment
            && self.applied_seq == other.applied_seq
            && self.segments() == other.segments()
    }
}

impl<const SEGMENTS: usize, const NAME: usize> Eq for State<SEGMENTS, NAME> {}

impl<const SEGMENTS: usize, const NAME: usize> fmt::Debug for State<SEGMENTS, NAME> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("brightness", &self.brightness)
            .field("last_brightness", &self.last_brightness)
            .field("transition", &self.transition)
            .field("main_segment", &self.main_segment)
            .field("applied_seq", &self.applied_seq)
            .field("segments", &self.segments())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestState = State<4, 16>;

    #[test]
    fn a_fresh_state_matches_a_freshly_booted_fixture() {
        let s = TestState::new(Layout::new(60));

        assert_eq!(s.brightness, 128);
        assert_eq!(s.last_brightness, 128);
        assert!(s.is_on());
        assert_eq!(s.transition.as_millis(), 750);
        assert_eq!(s.main_segment, 0);
        assert_eq!(s.applied_seq, Seq::ZERO);

        let [seg] = s.segments() else {
            panic!("expected exactly one segment, got {}", s.segments().len())
        };
        assert_eq!((seg.start, seg.stop), (0, 60), "covers every LED");
        assert!(seg.on && seg.selected);
        assert_eq!(seg.opacity, 255);
        assert_eq!(
            seg.colors,
            [Rgbw::new(0xFF, 0xA0, 0x00, 0), Rgbw::BLACK, Rgbw::BLACK],
            "primary warm orange, the rest black"
        );
        assert_eq!(seg.effect, EffectId(0));
        assert_eq!((seg.speed, seg.intensity), (128, 128));
        assert_eq!(seg.palette, PaletteId(0));
        assert!(!seg.reverse && !seg.mirror);
        assert!(seg.name.is_empty());
    }

    #[test]
    fn a_segment_is_never_empty() {
        let seg = Segment::<8>::new(5, 5);
        assert_eq!((seg.start, seg.stop), (5, 6));
        assert!(!seg.is_empty());
        assert_eq!(Segment::<8>::new(u16::MAX, 0).stop, u16::MAX, "saturates");
    }

    #[test]
    fn an_empty_strip_still_gets_a_one_led_segment() {
        let s = TestState::new(Layout::new(0));
        assert_eq!(s.segments()[0].len(), 1);
    }

    #[test]
    fn push_segment_respects_capacity() {
        let mut s = TestState::new(Layout::new(10));
        for expected in 1..4 {
            assert_eq!(s.push_segment(Segment::new(0, 1)), Ok(expected));
        }
        let extra = Segment::new(3, 4);
        assert_eq!(s.push_segment(extra), Err(extra));
        assert_eq!(s.segments().len(), s.segment_capacity());
    }

    #[test]
    fn equality_only_sees_active_segments() {
        let a = TestState::new(Layout::new(10));
        let b = TestState::new(Layout::new(10));
        assert_eq!(a, b);

        let mut c = b.clone();
        c.segments_mut()[0].speed = 1;
        assert_ne!(a, c);
    }

    #[test]
    fn has_applied_compares_sequence_numbers() {
        let mut s = TestState::new(Layout::new(10));
        s.applied_seq = Seq(5);
        assert!(s.has_applied(Seq(4)));
        assert!(s.has_applied(Seq(5)));
        assert!(!s.has_applied(Seq(6)));
    }
}
