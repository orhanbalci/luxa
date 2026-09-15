//! Fixture state: what the engine owns and publishes.

use core::fmt;

use luxa_color::Rgbw;

use crate::{EffectId, LightCaps, Name, PaletteId, Seq, TransitionTime};

/// The fixture a [`State`] describes.
///
/// Passed in when state is created rather than baked in as a constant, so the
/// same types serve any strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layout {
    /// Total number of addressable LEDs.
    pub led_count: u16,
    /// What those LEDs can show. One value for the whole fixture for now;
    /// per-range capabilities arrive with multiple outputs.
    pub caps: LightCaps,
}

impl Layout {
    /// A fixture of `led_count` full-colour RGB LEDs.
    pub const fn new(led_count: u16) -> Self {
        Self::with_caps(led_count, LightCaps::RGB)
    }

    /// A fixture of `led_count` LEDs that can show `caps`.
    pub const fn with_caps(led_count: u16, caps: LightCaps) -> Self {
        Self { led_count, caps }
    }
}

/// A contiguous range of LEDs running one effect with its own settings.
///
/// A segment is *active* while `stop > start`. Deleting one sets `stop` to `0`
/// but keeps its slot, so the ids of the segments after it do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Segment<const NAME: usize> {
    /// First LED, inclusive.
    pub start: u16,
    /// Last LED, exclusive. `0` once the segment is deleted.
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
    /// What the LEDs under this segment can show.
    pub caps: LightCaps,
    /// Display name; empty when unnamed.
    pub name: Name<NAME>,
}

impl<const NAME: usize> Segment<NAME> {
    /// Default effect speed.
    pub const DEFAULT_SPEED: u8 = 128;
    /// Default effect intensity.
    pub const DEFAULT_INTENSITY: u8 = 128;
    /// Warm orange: the primary colour of newly created segments, so a new
    /// segment visibly lights up.
    pub const DEFAULT_COLOR: Rgbw = Rgbw::from_u32(0xFFA000);

    /// A plain segment over `[start, stop)`: on, fully opaque, selected, all
    /// colours black, effect 0 at default speed and intensity, no known
    /// capabilities, unnamed.
    ///
    /// A `stop` not after `start` yields a one-LED segment — a new segment is
    /// never empty.
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
            caps: LightCaps::NONE,
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

    /// Whether the segment covers any LEDs, i.e. has not been deleted.
    pub const fn is_active(&self) -> bool {
        self.stop > self.start
    }

    /// Number of LEDs covered; `0` once deleted.
    pub const fn len(&self) -> u16 {
        self.stop.saturating_sub(self.start)
    }

    /// Whether the segment covers no LEDs, i.e. has been deleted.
    pub const fn is_empty(&self) -> bool {
        !self.is_active()
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
        segments[0].caps = layout.caps;
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

    /// Every segment slot in id order, deleted ones included — check
    /// [`Segment::is_active`], or use [`active_segments`](Self::active_segments).
    pub fn segments(&self) -> &[Segment<NAME>] {
        &self.segments[..self.segment_count]
    }

    /// Every segment slot, mutably.
    pub fn segments_mut(&mut self) -> &mut [Segment<NAME>] {
        &mut self.segments[..self.segment_count]
    }

    /// The segment with `id`, active or not.
    pub fn segment(&self, id: usize) -> Option<&Segment<NAME>> {
        self.segments().get(id)
    }

    /// The active segments with their ids.
    pub fn active_segments(&self) -> impl Iterator<Item = (usize, &Segment<NAME>)> {
        self.segments()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_active())
    }

    /// Appends a segment, returning its id — or the segment back if the state
    /// is at capacity.
    pub fn push_segment(&mut self, segment: Segment<NAME>) -> Result<usize, Segment<NAME>> {
        if self.segment_count == SEGMENTS {
            return Err(segment);
        }
        self.segments[self.segment_count] = segment;
        self.segment_count += 1;
        Ok(self.segment_count - 1)
    }

    /// Removes deleted segments after the first, closing the gaps so later ids
    /// shift down, and returns how many were removed. When any is removed the
    /// main segment falls back to the first.
    pub fn purge_inactive(&mut self) -> usize {
        // Slot 0 is never removed.
        let first = self.segment_count.min(1);
        let mut kept = first;
        let mut removed = 0;
        for i in first..self.segment_count {
            if self.segments[i].stop == 0 {
                removed += 1;
            } else {
                self.segments[kept] = self.segments[i];
                kept += 1;
            }
        }
        self.segment_count = kept;
        if removed > 0 {
            self.main_segment = 0;
        }
        removed
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
        assert!(seg.on && seg.selected && seg.is_active());
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
        assert_eq!(seg.caps, LightCaps::RGB, "from the layout");
        assert!(seg.name.is_empty());
    }

    #[test]
    fn a_new_segment_covers_at_least_one_led() {
        let seg = Segment::<8>::new(5, 5);
        assert_eq!((seg.start, seg.stop), (5, 6));
        assert!(seg.is_active());
        assert_eq!(Segment::<8>::new(u16::MAX, 0).stop, u16::MAX, "saturates");
    }

    #[test]
    fn a_deleted_segment_is_inactive_and_empty() {
        let mut seg = Segment::<8>::new(10, 20);
        seg.stop = 0;
        assert!(!seg.is_active());
        assert!(seg.is_empty());
        assert_eq!(seg.len(), 0, "no underflow");
    }

    #[test]
    fn an_empty_strip_still_gets_a_one_led_segment() {
        let s = TestState::new(Layout::new(0));
        assert_eq!(s.segments()[0].len(), 1);
    }

    #[test]
    fn layout_capabilities_reach_the_first_segment() {
        let s = TestState::new(Layout::with_caps(10, LightCaps::WHITE));
        assert_eq!(s.segments()[0].caps, LightCaps::WHITE);
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
    fn active_segments_skip_deleted_slots_but_keep_ids() {
        let mut s = TestState::new(Layout::new(30));
        s.push_segment(Segment::new(10, 20)).unwrap();
        s.push_segment(Segment::new(20, 30)).unwrap();
        s.segments_mut()[1].stop = 0;

        let ids: [usize; 2] = {
            let mut it = s.active_segments().map(|(id, _)| id);
            [it.next().unwrap(), it.next().unwrap()]
        };
        assert_eq!(ids, [0, 2]);
        assert_eq!(s.segments().len(), 3, "the deleted slot is still there");
    }

    #[test]
    fn purging_closes_gaps_but_keeps_the_first_slot() {
        let mut s = TestState::new(Layout::new(30));
        s.push_segment(Segment::new(10, 20)).unwrap();
        s.push_segment(Segment::new(20, 30)).unwrap();
        s.segments_mut()[0].stop = 0;
        s.segments_mut()[1].stop = 0;
        s.main_segment = 2;

        assert_eq!(s.purge_inactive(), 1, "slot 0 is never purged");
        assert_eq!(s.segments().len(), 2);
        assert_eq!(s.segments()[1].start, 20, "id 2 moved down to id 1");
        assert_eq!(s.main_segment, 0);
        assert_eq!(s.purge_inactive(), 0);
    }

    #[test]
    fn equality_sees_every_slot() {
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
