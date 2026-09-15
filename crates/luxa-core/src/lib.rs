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
//! use luxa_msg::{Command, Layout};
//!
//! let mut engine = Engine::<8, 32>::new(Layout::new(60));
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
//! use luxa_msg::{Command, Envelope, Layout, Seq};
//!
//! let mut engine = Engine::<8, 32>::new(Layout::new(60));
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

use luxa_msg::{BoolOp, Command, Envelope, GlobalPatch, Layout, Origins, State, U8Op};

/// Owns the fixture [`State`] and is the only thing that writes it.
///
/// `SEGMENTS` and `NAME` are the state's capacities; see [`State`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine<const SEGMENTS: usize, const NAME: usize> {
    layout: Layout,
    state: State<SEGMENTS, NAME>,
}

/// What a batch did, when there is something to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome<'a, const SEGMENTS: usize, const NAME: usize> {
    /// The state to publish.
    pub state: &'a State<SEGMENTS, NAME>,
    /// Where the batch's changes came from: the origin of every command that
    /// changed something. Empty when the batch is published only because a
    /// sender awaits a reply — so there is nothing to announce to peers.
    pub origins: Origins,
}

impl<const SEGMENTS: usize, const NAME: usize> Engine<SEGMENTS, NAME> {
    /// An engine for a freshly booted fixture.
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            state: State::new(layout),
        }
    }

    /// An engine restored to a known state — from persistence, in a later step.
    pub const fn with_state(layout: Layout, state: State<SEGMENTS, NAME>) -> Self {
        Self { layout, state }
    }

    /// The fixture this engine drives.
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// The current state, read-only.
    ///
    /// The runtime publishes a copy of this once at startup so the render task
    /// has something to draw before any command arrives.
    pub const fn state(&self) -> &State<SEGMENTS, NAME> {
        &self.state
    }

    /// Applies one command, returning whether it actually changed anything.
    ///
    /// Redundant commands report `false` so a batch of no-ops can skip its
    /// publish entirely.
    ///
    /// The engine currently applies brightness and power set to a value, and
    /// the default transition. Segment patches and relative values (toggle,
    /// step, random) are accepted but not yet applied.
    pub fn apply(&mut self, command: Command<NAME>) -> bool {
        match command {
            Command::Global(patch) => self.apply_global(patch),
            Command::Segment(_) => false,
        }
    }

    /// Applies a whole batch and returns what to publish, if anything.
    ///
    /// The batch can be bare [`Command`]s or numbered [`Envelope`]s. Returns
    /// `None` when nothing changed and nobody is waiting — there is no point
    /// waking the render path to tell it the world is exactly as it left it.
    /// `applied_seq` advances to the highest number in the batch either way;
    /// bare commands do not move it.
    pub fn apply_batch<E: Into<Envelope<NAME>>>(
        &mut self,
        batch: impl IntoIterator<Item = E>,
    ) -> Option<Outcome<'_, SEGMENTS, NAME>> {
        let mut origins = Origins::EMPTY;
        let mut awaited = false;
        for item in batch {
            let envelope: Envelope<NAME> = item.into();
            if self.apply(envelope.command) {
                origins = origins.with(envelope.origin);
            }
            awaited |= envelope.awaits_reply;
            self.state.applied_seq = self.state.applied_seq.max(envelope.seq);
        }
        (awaited || !origins.is_empty()).then_some(Outcome {
            state: &self.state,
            origins,
        })
    }

    fn apply_global(&mut self, patch: GlobalPatch) -> bool {
        let mut changed = false;
        // Brightness before power, so a patch that sets a level and switches
        // off remembers that level for switching back on.
        if let Some(U8Op::Set(level)) = patch.brightness {
            changed |= self.set_brightness(level);
        }
        if let Some(BoolOp::Set(on)) = patch.on {
            changed |= self.set_power(on);
        }
        if let Some(transition) = patch.transition {
            changed |= self.state.transition != transition;
            self.state.transition = transition;
        }
        changed
    }

    fn set_power(&mut self, on: bool) -> bool {
        let s = &mut self.state;
        if on == s.is_on() {
            return false;
        }
        if on {
            s.brightness = s.last_brightness;
        } else {
            // Remember the level so switching back on restores it.
            s.last_brightness = s.brightness;
            s.brightness = 0;
        }
        on == s.is_on()
    }

    fn set_brightness(&mut self, level: u8) -> bool {
        let s = &mut self.state;
        let changed = s.brightness != level || (level > 0 && s.last_brightness != level);
        s.brightness = level;
        // Zero is "off", not a level to come back to.
        if level > 0 {
            s.last_brightness = level;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_msg::{Origin, SegmentPatch, Seq, TransitionTime};

    type TestEngine = Engine<4, 16>;
    type Cmd = Command<16>;

    const LAYOUT: Layout = Layout::new(30);

    fn env(seq: u32, command: Cmd) -> Envelope<16> {
        Envelope::new(Seq(seq), command)
    }

    #[test]
    fn starts_in_a_freshly_booted_state() {
        let engine = TestEngine::new(LAYOUT);
        assert_eq!(engine.state(), &State::new(LAYOUT));
        assert_eq!(engine.layout(), LAYOUT);
    }

    #[test]
    fn power_off_remembers_brightness_and_power_on_restores_it() {
        let mut engine = TestEngine::new(LAYOUT);
        assert!(engine.apply(Cmd::brightness(200)));
        assert!(engine.apply(Cmd::power(false)));
        assert!(!engine.state().is_on());
        assert_eq!(engine.state().brightness, 0);
        assert_eq!(engine.state().last_brightness, 200);

        assert!(engine.apply(Cmd::power(true)));
        assert_eq!(engine.state().brightness, 200);
    }

    #[test]
    fn setting_brightness_while_off_switches_on() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply(Cmd::power(false));
        assert!(engine.apply(Cmd::brightness(42)));
        assert!(engine.state().is_on());
        assert_eq!(engine.state().brightness, 42);
    }

    #[test]
    fn brightness_zero_switches_off_and_keeps_the_last_level() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply(Cmd::brightness(90));
        assert!(engine.apply(Cmd::brightness(0)));
        assert!(!engine.state().is_on());
        assert_eq!(engine.state().last_brightness, 90);

        engine.apply(Cmd::power(true));
        assert_eq!(engine.state().brightness, 90);
    }

    #[test]
    fn brightness_is_applied_before_power() {
        let mut engine = TestEngine::new(LAYOUT);
        let patch = GlobalPatch {
            brightness: Some(U8Op::Set(50)),
            on: Some(BoolOp::Set(false)),
            ..GlobalPatch::NONE
        };
        assert!(engine.apply(Cmd::Global(patch)));
        assert!(!engine.state().is_on());
        assert_eq!(engine.state().last_brightness, 50);
    }

    #[test]
    fn transition_sets_the_default_duration() {
        let mut engine = TestEngine::new(LAYOUT);
        let one_second = TransitionTime::from_deciseconds(10);
        let patch = GlobalPatch {
            transition: Some(one_second),
            ..GlobalPatch::NONE
        };
        assert!(engine.apply(Cmd::Global(patch)));
        assert_eq!(engine.state().transition, one_second);
        assert!(!engine.apply(Cmd::Global(patch)), "already that duration");
    }

    #[test]
    fn redundant_commands_report_no_change() {
        let mut engine = TestEngine::new(LAYOUT);
        assert!(!engine.apply(Cmd::power(true)), "already on");
        assert!(engine.apply(Cmd::brightness(7)));
        assert!(!engine.apply(Cmd::brightness(7)));
        assert!(engine.apply(Cmd::power(false)));
        assert!(!engine.apply(Cmd::power(false)), "already off");
        assert!(!engine.apply(Cmd::brightness(0)), "already off");
        assert!(!engine.apply(Cmd::Global(GlobalPatch::NONE)));
    }

    #[test]
    fn segment_patches_are_accepted() {
        let mut engine = TestEngine::new(LAYOUT);
        let before = engine.state().clone();
        engine.apply(Cmd::Segment(SegmentPatch::for_selected()));
        assert_eq!(engine.state(), &before);
    }

    #[test]
    fn a_batch_publishes_once_with_the_final_value() {
        let mut engine = TestEngine::new(LAYOUT);
        let published = engine
            .apply_batch([
                Cmd::brightness(10),
                Cmd::brightness(20),
                Cmd::power(false),
                Cmd::brightness(30),
            ])
            .expect("state changed, so a publish is due")
            .state
            .clone();
        assert_eq!(published.brightness, 30);
        assert_eq!(&published, engine.state());
    }

    #[test]
    fn an_empty_batch_publishes_nothing() {
        assert_eq!(TestEngine::new(LAYOUT).apply_batch::<Cmd>([]), None);
    }

    #[test]
    fn a_batch_of_no_ops_publishes_nothing() {
        let mut engine = TestEngine::new(LAYOUT);
        let level = engine.state().brightness;
        assert_eq!(
            engine.apply_batch([Cmd::power(true), Cmd::brightness(level)]),
            None
        );
    }

    #[test]
    fn bare_commands_leave_applied_seq_alone() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply_batch([env(5, Cmd::brightness(1))]);
        engine.apply_batch([Cmd::brightness(2)]);
        assert_eq!(engine.state().brightness, 2, "applied like any other");
        assert_eq!(engine.state().applied_seq, Seq(5), "but not numbered");
    }

    #[test]
    fn a_batch_that_nets_out_to_no_change_still_publishes() {
        // Intermediate states are not observable, but the engine must not try
        // to be clever about it: any step that moved counts, and reporting a
        // redundant publish is far safer than dropping a real one.
        let mut engine = TestEngine::new(LAYOUT);
        let before = engine.state().brightness;
        let published = engine.apply_batch([Cmd::brightness(1), Cmd::brightness(before)]);
        assert_eq!(published.map(|o| o.state.brightness), Some(before));
    }

    #[test]
    fn every_command_in_a_batch_is_applied() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply_batch([Cmd::brightness(3), Cmd::power(false)]);
        assert_eq!(engine.state().brightness, 0);
        assert_eq!(engine.state().last_brightness, 3);
    }

    #[test]
    fn a_no_op_awaiting_a_reply_still_publishes_and_advances_applied_seq() {
        let mut engine = TestEngine::new(LAYOUT);
        let published = engine
            .apply_batch([Envelope::awaiting_reply(Seq(1), Cmd::power(true))])
            .expect("someone is waiting, so a publish is due even without a change");
        assert_eq!(published.state.applied_seq, Seq(1));
        assert!(published.state.has_applied(Seq(1)));
        assert!(
            published.origins.is_empty(),
            "nothing changed, nothing to announce"
        );
    }

    #[test]
    fn applied_seq_advances_even_when_nothing_is_published() {
        let mut engine = TestEngine::new(LAYOUT);
        assert_eq!(engine.apply_batch([env(7, Cmd::power(true))]), None);
        assert_eq!(engine.state().applied_seq, Seq(7));
    }

    #[test]
    fn a_waiter_is_not_satisfied_by_an_earlier_batch() {
        let mut engine = TestEngine::new(LAYOUT);
        // The waiter's command is Seq(3), still queued behind this batch.
        let early = engine
            .apply_batch([env(1, Cmd::brightness(10)), env(2, Cmd::brightness(20))])
            .expect("state changed");
        assert!(!early.state.has_applied(Seq(3)));

        let late = engine
            .apply_batch([Envelope::awaiting_reply(Seq(3), Cmd::brightness(20))])
            .expect("awaited");
        assert!(late.state.has_applied(Seq(3)));
        assert_eq!(late.state.brightness, 20);
    }

    #[test]
    fn origins_record_only_commands_that_changed_something() {
        let mut engine = TestEngine::new(LAYOUT);
        let outcome = engine
            .apply_batch([
                env(1, Cmd::brightness(10)).with_origin(Origin::Notification),
                env(2, Cmd::power(true)).with_origin(Origin::Button), // already on
            ])
            .expect("state changed");
        assert!(outcome.origins.contains(Origin::Notification));
        assert!(!outcome.origins.contains(Origin::Button));
        assert!(
            !outcome.origins.notifies_peers(),
            "a change received from a peer must not be echoed back"
        );
    }

    #[test]
    fn a_direct_change_is_announced() {
        let mut engine = TestEngine::new(LAYOUT);
        let outcome = engine.apply_batch([Cmd::brightness(10)]).expect("changed");
        assert!(outcome.origins.contains(Origin::Direct));
        assert!(outcome.origins.notifies_peers());
    }

    #[test]
    fn state_can_be_restored() {
        let mut state = State::new(LAYOUT);
        state.brightness = 0;
        state.last_brightness = 9;
        let engine = TestEngine::with_state(LAYOUT, state.clone());
        assert_eq!(engine.state(), &state);
    }
}
