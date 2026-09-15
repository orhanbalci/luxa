//! The engine — the single writer of fixture state.
//!
//! Every ingress (HTTP today; MQTT, buttons and schedules later) produces
//! [`Envelope`]s onto one channel. This crate drains that channel, applies the
//! commands, and publishes a [`State`]. Nothing else in Luxa mutates state,
//! which is what makes "what is the fixture doing right now?" a question with
//! exactly one answer.
//!
//! It knows nothing about sockets, executors or pixels. There is no `async`
//! here and no channel type — the runtime owns those and calls in. That is why
//! every test below is a plain synchronous test with no executor to spin up.
//!
//! # Drain, then publish
//!
//! [`Engine::apply_batch`] is the real loop, not a shortcut. The runtime drains
//! everything currently queued, applies it, and publishes *one* state for the
//! batch. This matters as soon as there is a slider in the UI: dragging it
//! emits commands far faster than frames render, and publishing per command
//! would make the renderer chase a backlog it can never catch up with.
//! Coalescing means a burst of forty brightness commands costs one publish and
//! the renderer always sees the newest value.
//!
//! ```
//! use luxa_core::Engine;
//! use luxa_msg::{Catalogue, Command, Layout};
//!
//! let mut engine = Engine::<8, 32>::new(Layout::new(60), Catalogue::contiguous(1, 1));
//! let published = engine.apply_batch([
//!     Command::brightness(10),
//!     Command::brightness(20),
//!     Command::brightness(30),
//! ]);
//!
//! // Three commands in, one state out, carrying only the final value.
//! assert_eq!(published.unwrap().state.brightness, 30);
//! ```
//!
//! # Acknowledgement
//!
//! Most callers never see a sequence number: a bare [`Command`] is all
//! `apply_batch` needs. Numbering exists for one case — a sender that needs the
//! state *its* commands produced, such as an HTTP request that must answer with
//! the new state, while other producers share the same queue.
//!
//! Such a sender numbers its commands at enqueue time, marks the last one
//! [`awaiting_reply`](Envelope::awaiting_reply), and waits for a published
//! state that [`has_applied`](State::has_applied) that number. The engine
//! publishes for a batch containing such a command *even if nothing changed* —
//! otherwise "turn on" sent to a fixture that is already on would never be
//! answered.
//!
//! ```
//! use luxa_core::Engine;
//! use luxa_msg::{Catalogue, Command, Envelope, Layout, Seq};
//!
//! let mut engine = Engine::<8, 32>::new(Layout::new(60), Catalogue::contiguous(1, 1));
//! let mine = Seq(1);
//!
//! // Already on, so nothing changes — but someone is waiting, so it publishes.
//! let reply = engine
//!     .apply_batch([Envelope::awaiting_reply(mine, Command::power(true))])
//!     .expect("an awaited command always publishes");
//! assert!(reply.state.has_applied(mine));
//! ```

#![no_std]
#![forbid(unsafe_code)]

mod resolve;

use core::ops::{BitOr, BitOrAssign};

use luxa_color::{Chsv, hsv2rgb_rainbow, kelvin_to_rgb};
use luxa_msg::{
    BoolOp, Catalogue, ColorSpec, Command, EffectId, Envelope, GlobalPatch, Layout, LightCaps,
    Name, Origins, PaletteId, Rgbw, Segment, SegmentPatch, SegmentTarget, State,
};

use crate::resolve::{Range, Rng, resolve_bool, resolve_u8};

/// What a command or batch changed, by kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Changes(u16);

impl Changes {
    /// Nothing changed.
    pub const NONE: Self = Self(0);
    /// Master brightness or power.
    pub const BRIGHTNESS: Self = Self(1 << 0);
    /// The default transition duration.
    pub const TRANSITION: Self = Self(1 << 1);
    /// A segment's opacity.
    pub const SEGMENT_OPACITY: Self = Self(1 << 2);
    /// A segment's power, reverse or mirror option.
    pub const SEGMENT_OPTIONS: Self = Self(1 << 3);
    /// A segment's colours.
    pub const SEGMENT_COLORS: Self = Self(1 << 4);
    /// A segment's effect, speed, intensity or palette.
    pub const SEGMENT_EFFECT: Self = Self(1 << 5);
    /// Segment bounds — including creating, deleting or compacting segments.
    pub const SEGMENT_BOUNDS: Self = Self(1 << 6);
    /// Which segments are selected.
    pub const SEGMENT_SELECTION: Self = Self(1 << 7);
    /// A segment's name.
    pub const SEGMENT_NAME: Self = Self(1 << 8);

    const ORGANISATION: u16 = Self::SEGMENT_SELECTION.0 | Self::SEGMENT_NAME.0;

    /// Whether anything changed.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every change in `other` is in this set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether this changes what the fixture shows or does.
    ///
    /// Selecting and naming segments only reorganise how a UI presents the
    /// fixture: they are published so reads stay current, but are not
    /// announced as changes.
    pub const fn is_state_change(self) -> bool {
        self.0 & !Self::ORGANISATION != 0
    }
}

impl BitOr for Changes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Changes {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What a batch did, when there is something to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome<'a, const SEGMENTS: usize, const NAME: usize> {
    /// The state to publish.
    pub state: &'a State<SEGMENTS, NAME>,
    /// Where the batch's state changes came from: the origin of every command
    /// whose changes [are state changes](Changes::is_state_change). Empty when
    /// there is nothing to announce to peers.
    pub origins: Origins,
    /// Everything the batch changed.
    pub changes: Changes,
}

/// Owns the fixture [`State`] and is the only thing that writes it.
///
/// `SEGMENTS` and `NAME` are the state's capacities; see [`State`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine<const SEGMENTS: usize, const NAME: usize> {
    layout: Layout,
    catalogue: Catalogue,
    rng: Rng,
    state: State<SEGMENTS, NAME>,
}

impl<const SEGMENTS: usize, const NAME: usize> Engine<SEGMENTS, NAME> {
    const DEFAULT_SEED: u32 = 0x4C55_5841;

    /// An engine for a freshly booted fixture offering `catalogue`.
    pub fn new(layout: Layout, catalogue: Catalogue) -> Self {
        Self::with_state(layout, catalogue, State::new(layout))
    }

    /// An engine restored to a known state — from persistence, in a later step.
    pub const fn with_state(
        layout: Layout,
        catalogue: Catalogue,
        state: State<SEGMENTS, NAME>,
    ) -> Self {
        Self {
            layout,
            catalogue,
            rng: Rng::new(Self::DEFAULT_SEED),
            state,
        }
    }

    /// The same engine with its random choices seeded from `seed`.
    ///
    /// The runtime seeds from a hardware source; tests pick a fixed seed so
    /// random values are reproducible.
    #[must_use]
    pub const fn with_seed(mut self, seed: u32) -> Self {
        self.rng = Rng::new(seed);
        self
    }

    /// The fixture this engine drives.
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// The effects and palettes this engine accepts.
    pub const fn catalogue(&self) -> Catalogue {
        self.catalogue
    }

    /// The current state, read-only.
    ///
    /// The runtime publishes a copy of this once at startup so the render task
    /// has something to draw before any command arrives.
    pub const fn state(&self) -> &State<SEGMENTS, NAME> {
        &self.state
    }

    /// Applies one command and reports what it changed.
    ///
    /// Redundant commands report [`Changes::NONE`] so a batch of no-ops can skip
    /// its publish entirely.
    ///
    /// The command is a change of its own: [`State::change_transition`] returns
    /// to the default duration unless the command sets a one-shot one.
    pub fn apply(&mut self, command: Command<NAME>) -> Changes {
        self.state.change_transition = self.state.transition;
        self.apply_command(command)
    }

    fn apply_command(&mut self, command: Command<NAME>) -> Changes {
        match command {
            Command::Global(patch) => self.apply_global(patch),
            Command::Segment(patch) => self.apply_segment(&patch),
            Command::CompactSegments { deleted } => self.compact_segments(deleted),
        }
    }

    /// Applies a whole batch and returns what to publish, if anything.
    ///
    /// The batch can be bare [`Command`]s or numbered [`Envelope`]s. Returns
    /// `None` when nothing changed and nobody is waiting — there is no point
    /// waking the render path to tell it the world is exactly as it left it.
    /// `applied_seq` advances to the highest number in the batch either way;
    /// bare commands do not move it.
    ///
    /// The batch is one change: a one-shot transition duration any of its
    /// commands sets applies to all of it, in [`State::change_transition`].
    pub fn apply_batch<E: Into<Envelope<NAME>>>(
        &mut self,
        batch: impl IntoIterator<Item = E>,
    ) -> Option<Outcome<'_, SEGMENTS, NAME>> {
        let mut origins = Origins::EMPTY;
        let mut changes = Changes::NONE;
        let mut awaited = false;
        self.state.change_transition = self.state.transition;
        for item in batch {
            let envelope: Envelope<NAME> = item.into();
            let applied = self.apply_command(envelope.command);
            if applied.is_state_change() {
                origins = origins.with(envelope.origin);
            }
            changes |= applied;
            awaited |= envelope.awaits_reply;
            self.state.applied_seq = self.state.applied_seq.max(envelope.seq);
        }
        (awaited || !changes.is_empty()).then_some(Outcome {
            state: &self.state,
            origins,
            changes,
        })
    }

    fn apply_global(&mut self, patch: GlobalPatch) -> Changes {
        let mut changes = Changes::NONE;
        let before = (self.state.brightness, self.state.last_brightness);
        let was_on = self.state.is_on();

        // Brightness first, so a patch that sets a level and switches off
        // remembers that level for switching back on.
        if let Some(op) = patch.brightness {
            self.state.brightness =
                resolve_u8(op, self.state.brightness, Range::FULL, &mut self.rng);
        }
        // Power follows the brightness unless set explicitly: nonzero is on.
        let on = match patch.on {
            Some(BoolOp::Set(on)) => on,
            _ => self.state.is_on(),
        };
        if on != self.state.is_on() {
            self.toggle_power();
        }
        // A toggle flips the power the fixture had before this patch — unless
        // the patch's brightness has just switched it on.
        if patch.on == Some(BoolOp::Toggle) && (was_on || !self.state.is_on()) {
            self.toggle_power();
        }
        if self.state.brightness > 0 {
            self.state.last_brightness = self.state.brightness;
        }
        if (self.state.brightness, self.state.last_brightness) != before {
            changes |= Changes::BRIGHTNESS;
        }

        if let Some(transition) = patch.transition {
            if transition != self.state.transition {
                self.state.transition = transition;
                changes |= Changes::TRANSITION;
            }
        }
        // How long this change takes to show: a one-shot duration if the patch
        // has one, otherwise the default it may just have set. Neither is a
        // state change on its own.
        match patch.transition_once {
            Some(once) => self.state.change_transition = once,
            None if patch.transition.is_some() => {
                self.state.change_transition = self.state.transition;
            }
            None => {}
        }
        changes
    }

    fn toggle_power(&mut self) {
        let s = &mut self.state;
        if s.brightness == 0 {
            s.brightness = s.last_brightness;
        } else {
            s.last_brightness = s.brightness;
            s.brightness = 0;
        }
    }

    fn apply_segment(&mut self, patch: &SegmentPatch<NAME>) -> Changes {
        match patch.target {
            SegmentTarget::Id(id) => self.apply_to_segment(usize::from(id), patch),
            SegmentTarget::Selected => {
                let mut changes = Changes::NONE;
                for id in 0..self.state.segments().len() {
                    let seg = &self.state.segments()[id];
                    if seg.is_active() && seg.selected {
                        changes |= self.apply_to_segment(id, patch);
                    }
                }
                changes
            }
        }
    }

    fn apply_to_segment(&mut self, id: usize, patch: &SegmentPatch<NAME>) -> Changes {
        if id >= SEGMENTS {
            return Changes::NONE;
        }
        let led_count = self.layout.led_count;
        let mut changes = Changes::NONE;

        // An id past the last segment creates one — but only with a stop.
        let id = if id >= self.state.segments().len() {
            if !matches!(patch.stop, Some(stop) if stop > 0) {
                return Changes::NONE;
            }
            let mut created = Segment::new(0, led_count);
            created.colors[0] = Segment::<NAME>::DEFAULT_COLOR;
            created.caps = self.layout.caps;
            // Appended at the end, whatever id was asked for.
            let Ok(new_id) = self.state.push_segment(created) else {
                return Changes::NONE;
            };
            changes |= Changes::SEGMENT_BOUNDS;
            new_id
        } else {
            id
        };

        let old = self.state.segments()[id];
        let mut seg = old;

        let start = patch.start.unwrap_or(old.start);
        let stop = match (patch.stop, patch.len) {
            (Some(stop), _) => stop,
            (None, Some(len)) if len > 0 => start.saturating_add(len),
            _ => old.stop,
        };
        // An explicit name wins; otherwise moving the bounds clears the name.
        if let Some(name) = patch.name {
            seg.name = name;
        } else if (start, stop) != (old.start, old.stop) {
            seg.name = Name::EMPTY;
        }
        (seg.start, seg.stop) = sanitize_bounds(start, stop, old.start, led_count);
        if seg.is_active() {
            seg.caps = self.layout.caps;
        }
        if (seg.start, seg.stop) != (old.start, old.stop) {
            changes |= Changes::SEGMENT_BOUNDS;
        }
        if seg.name != old.name {
            changes |= Changes::SEGMENT_NAME;
        }

        if !seg.is_active() {
            // Deleted (or already gone): nothing else applies to it.
            if id == usize::from(self.state.main_segment) {
                self.state.main_segment = 0;
            }
            self.state.segments_mut()[id] = seg;
            return changes;
        }

        // A nonzero opacity switches the segment on; zero switches it off and
        // keeps the opacity it had.
        if let Some(op) = patch.opacity {
            let level = resolve_u8(op, seg.opacity, Range::FULL, &mut self.rng);
            if level > 0 {
                seg.opacity = level;
            }
            seg.on = level > 0;
        }
        if let Some(op) = patch.on {
            seg.on = resolve_bool(op, seg.on);
        }

        if patch.colors.iter().any(Option::is_some) {
            if seg.caps.contains(LightCaps::RGB) || seg.caps.contains(LightCaps::WHITE) {
                for (slot, spec) in patch.colors.iter().enumerate() {
                    if let Some(spec) = *spec {
                        seg.colors[slot] = self.resolve_color(spec, seg.colors[slot]);
                    }
                }
            } else {
                // Neither colour nor white (a relay, say): colour means "on".
                seg.colors[0] = Rgbw::from_u32(u32::MAX);
                seg.colors[1] = Rgbw::BLACK;
            }
        }

        if let Some(op) = patch.selected {
            seg.selected = resolve_bool(op, seg.selected);
        }
        if let Some(op) = patch.reverse {
            seg.reverse = resolve_bool(op, seg.reverse);
        }
        if let Some(op) = patch.mirror {
            seg.mirror = resolve_bool(op, seg.mirror);
        }

        if let Some(op) = patch.effect {
            let count = self.catalogue.effects.end();
            let fx = resolve_u8(op, seg.effect.0, Range::up_to(count), &mut self.rng);
            if fx != seg.effect.0 {
                seg.effect = EffectId(self.valid_effect(fx));
            }
        }
        if let Some(op) = patch.speed {
            seg.speed = resolve_u8(op, seg.speed, Range::FULL, &mut self.rng);
        }
        if let Some(op) = patch.intensity {
            seg.intensity = resolve_u8(op, seg.intensity, Range::FULL, &mut self.rng);
        }
        // A palette means nothing to LEDs that cannot show colour.
        if let Some(op) = patch.palette {
            if seg.caps.contains(LightCaps::RGB) {
                let count = self.catalogue.palettes.end();
                let pal = resolve_u8(op, seg.palette.0, Range::up_to(count), &mut self.rng);
                let valid = self.catalogue.palettes.contains(pal);
                seg.palette = PaletteId(if valid { pal } else { 0 });
            }
        }
        for (slot, op) in patch.custom.iter().enumerate() {
            if let Some(op) = *op {
                let max = if slot == 2 {
                    Segment::<NAME>::CUSTOM3_MAX
                } else {
                    u8::MAX
                };
                let value = resolve_u8(
                    op,
                    seg.custom[slot],
                    Range::up_to(max.into()),
                    &mut self.rng,
                );
                seg.custom[slot] = value.min(max);
            }
        }
        for (slot, op) in patch.checks.iter().enumerate() {
            if let Some(op) = *op {
                seg.checks[slot] = resolve_bool(op, seg.checks[slot]);
            }
        }

        if seg.opacity != old.opacity {
            changes |= Changes::SEGMENT_OPACITY;
        }
        if (seg.on, seg.reverse, seg.mirror) != (old.on, old.reverse, old.mirror) {
            changes |= Changes::SEGMENT_OPTIONS;
        }
        if seg.colors != old.colors {
            changes |= Changes::SEGMENT_COLORS;
        }
        if (
            seg.effect,
            seg.speed,
            seg.intensity,
            seg.palette,
            seg.custom,
            seg.checks,
        ) != (
            old.effect,
            old.speed,
            old.intensity,
            old.palette,
            old.custom,
            old.checks,
        ) {
            changes |= Changes::SEGMENT_EFFECT;
        }
        if seg.selected != old.selected {
            changes |= Changes::SEGMENT_SELECTION;
        }

        self.state.segments_mut()[id] = seg;
        changes
    }

    /// The effect an id selects: gaps skip forward to the next effect, and
    /// anything past the last effect falls back to `0`.
    fn valid_effect(&self, fx: u8) -> u8 {
        let end = self.catalogue.effects.end();
        let mut id = u16::from(fx);
        while id < end && !self.catalogue.effects.contains(id as u8) {
            id += 1;
        }
        if id >= end { 0 } else { id as u8 }
    }

    fn resolve_color(&mut self, spec: ColorSpec, current: Rgbw) -> Rgbw {
        match spec {
            ColorSpec::Rgbw(color) => color,
            ColorSpec::Partial { r, g, b, w } => Rgbw::new(
                r.unwrap_or(current.r),
                g.unwrap_or(current.g),
                b.unwrap_or(current.b),
                w.unwrap_or(current.w),
            ),
            ColorSpec::Kelvin(0) => Rgbw::BLACK,
            ColorSpec::Kelvin(kelvin) => Rgbw::from_rgb(kelvin_to_rgb(kelvin)),
            ColorSpec::Random => {
                let hue = self.rng.next_u32() as u8;
                Rgbw::from_rgb(hsv2rgb_rainbow(Chsv::new(hue, 255, 255)))
            }
        }
    }

    fn compact_segments(&mut self, deleted: u8) -> Changes {
        let count = self.state.segments().len();
        // Only a run that deleted at least half of more than three segments
        // compacts; smaller deletions leave every id where it was.
        if count > 3 && usize::from(deleted) >= count / 2 && self.state.purge_inactive() > 0 {
            Changes::SEGMENT_BOUNDS
        } else {
            Changes::NONE
        }
    }
}

/// Bounds as a strip of `led_count` LEDs accepts them.
///
/// A stop at or before the start deletes the segment (stop `0`); a start past
/// the strip keeps the old start; a stop past the strip is clamped to it.
fn sanitize_bounds(start: u16, stop: u16, old_start: u16, led_count: u16) -> (u16, u16) {
    let mut stop = if stop <= start { 0 } else { stop };
    let start = if start >= led_count { old_start } else { start };
    if stop > led_count {
        stop = led_count;
    }
    if start >= stop {
        stop = 0;
    }
    (start, stop)
}

#[cfg(test)]
mod tests;
