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
//!     Command::Brightness(10),
//!     Command::Brightness(20),
//!     Command::Brightness(30),
//! ]);
//!
//! // Three commands in, one state out, carrying only the final value.
//! assert_eq!(published.unwrap().brightness, 30);
//! ```
//!
//! # Acknowledgement
//!
//! Most callers never see a sequence number: a bare [`Command`] is all
//! `apply_batch` needs. Numbering exists for one case — a sender that needs the
//! state *its* command produced, such as an HTTP request that must answer with
//! the new state, while other producers share the same queue.
//!
//! Such a sender numbers its command at enqueue time, marks it
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
//!     .apply_batch([Envelope::awaiting_reply(mine, Command::Power(true))])
//!     .expect("an awaited command always publishes");
//! assert!(reply.has_applied(mine));
//! ```

#![no_std]
#![forbid(unsafe_code)]

use luxa_msg::{Command, Envelope, Layout, State};

/// Owns the fixture [`State`] and is the only thing that writes it.
///
/// `SEGMENTS` and `NAME` are the state's capacities; see [`State`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine<const SEGMENTS: usize, const NAME: usize> {
    layout: Layout,
    state: State<SEGMENTS, NAME>,
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
    pub fn apply(&mut self, command: Command) -> bool {
        let s = &mut self.state;
        match command {
            Command::Power(true) => {
                if s.is_on() {
                    return false;
                }
                s.brightness = s.last_brightness;
                s.is_on()
            }
            Command::Power(false) => {
                if !s.is_on() {
                    return false;
                }
                // Remember the level so switching back on restores it.
                s.last_brightness = s.brightness;
                s.brightness = 0;
                true
            }
            Command::Brightness(level) => {
                let changed = s.brightness != level || (level > 0 && s.last_brightness != level);
                s.brightness = level;
                // Zero is "off", not a level to come back to.
                if level > 0 {
                    s.last_brightness = level;
                }
                changed
            }
        }
    }

    /// Applies a whole batch and returns the state to publish, if any.
    ///
    /// The batch can be bare [`Command`]s or numbered [`Envelope`]s. Returns
    /// `None` when nothing changed and nobody is waiting — there is no point
    /// waking the render path to tell it the world is exactly as it left it.
    /// `applied_seq` advances to the highest number in the batch either way;
    /// bare commands do not move it.
    pub fn apply_batch<E: Into<Envelope>>(
        &mut self,
        batch: impl IntoIterator<Item = E>,
    ) -> Option<&State<SEGMENTS, NAME>> {
        let mut changed = false;
        let mut awaited = false;
        for envelope in batch.into_iter().map(Into::into) {
            // Not `||`, which would short-circuit and stop applying.
            changed |= self.apply(envelope.command);
            awaited |= envelope.awaits_reply;
            self.state.applied_seq = self.state.applied_seq.max(envelope.seq);
        }
        (changed || awaited).then_some(&self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_msg::Seq;

    type TestEngine = Engine<4, 16>;

    const LAYOUT: Layout = Layout::new(30);

    fn env(seq: u32, command: Command) -> Envelope {
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
        assert!(engine.apply(Command::Brightness(200)));
        assert!(engine.apply(Command::Power(false)));
        assert!(!engine.state().is_on());
        assert_eq!(engine.state().brightness, 0);
        assert_eq!(engine.state().last_brightness, 200);

        assert!(engine.apply(Command::Power(true)));
        assert_eq!(engine.state().brightness, 200);
    }

    #[test]
    fn setting_brightness_while_off_switches_on() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply(Command::Power(false));
        assert!(engine.apply(Command::Brightness(42)));
        assert!(engine.state().is_on());
        assert_eq!(engine.state().brightness, 42);
    }

    #[test]
    fn brightness_zero_switches_off_and_keeps_the_last_level() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply(Command::Brightness(90));
        assert!(engine.apply(Command::Brightness(0)));
        assert!(!engine.state().is_on());
        assert_eq!(engine.state().last_brightness, 90);

        engine.apply(Command::Power(true));
        assert_eq!(engine.state().brightness, 90);
    }

    #[test]
    fn redundant_commands_report_no_change() {
        let mut engine = TestEngine::new(LAYOUT);
        assert!(!engine.apply(Command::Power(true)), "already on");
        assert!(engine.apply(Command::Brightness(7)));
        assert!(!engine.apply(Command::Brightness(7)));
        assert!(engine.apply(Command::Power(false)));
        assert!(!engine.apply(Command::Power(false)), "already off");
        assert!(!engine.apply(Command::Brightness(0)), "already off");
    }

    #[test]
    fn a_batch_publishes_once_with_the_final_value() {
        let mut engine = TestEngine::new(LAYOUT);
        let published = engine
            .apply_batch([
                Command::Brightness(10),
                Command::Brightness(20),
                Command::Power(false),
                Command::Brightness(30),
            ])
            .expect("state changed, so a publish is due")
            .clone();
        assert_eq!(published.brightness, 30);
        assert_eq!(&published, engine.state());
    }

    #[test]
    fn an_empty_batch_publishes_nothing() {
        assert_eq!(TestEngine::new(LAYOUT).apply_batch::<Command>([]), None);
    }

    #[test]
    fn a_batch_of_no_ops_publishes_nothing() {
        let mut engine = TestEngine::new(LAYOUT);
        let level = engine.state().brightness;
        assert_eq!(
            engine.apply_batch([Command::Power(true), Command::Brightness(level)]),
            None
        );
    }

    #[test]
    fn bare_commands_leave_applied_seq_alone() {
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply_batch([env(5, Command::Brightness(1))]);
        engine.apply_batch([Command::Brightness(2)]);
        assert_eq!(engine.state().brightness, 2, "applied like any other");
        assert_eq!(engine.state().applied_seq, Seq(5), "but not numbered");
    }

    #[test]
    fn a_batch_that_nets_out_to_no_change_still_publishes() {
        // Intermediate states are not observable, but the engine must not try
        // to be clever about it: `changed` tracks whether any step moved, and
        // reporting a redundant publish is far safer than dropping a real one.
        let mut engine = TestEngine::new(LAYOUT);
        let before = engine.state().brightness;
        let published = engine.apply_batch([Command::Brightness(1), Command::Brightness(before)]);
        assert_eq!(published.map(|s| s.brightness), Some(before));
    }

    #[test]
    fn every_command_in_a_batch_is_applied() {
        // Regression guard: a `||` here instead of `|=` would short-circuit
        // after the first change and silently drop the rest of the batch.
        let mut engine = TestEngine::new(LAYOUT);
        engine.apply_batch([Command::Brightness(3), Command::Power(false)]);
        assert_eq!(engine.state().brightness, 0);
        assert_eq!(engine.state().last_brightness, 3);
    }

    #[test]
    fn a_no_op_awaiting_a_reply_still_publishes_and_advances_applied_seq() {
        let mut engine = TestEngine::new(LAYOUT);
        let published = engine
            .apply_batch([Envelope::awaiting_reply(Seq(1), Command::Power(true))])
            .expect("someone is waiting, so a publish is due even without a change");
        assert_eq!(published.applied_seq, Seq(1));
        assert!(published.has_applied(Seq(1)));
    }

    #[test]
    fn applied_seq_advances_even_when_nothing_is_published() {
        let mut engine = TestEngine::new(LAYOUT);
        assert_eq!(engine.apply_batch([env(7, Command::Power(true))]), None);
        assert_eq!(engine.state().applied_seq, Seq(7));
    }

    #[test]
    fn a_waiter_is_not_satisfied_by_an_earlier_batch() {
        let mut engine = TestEngine::new(LAYOUT);
        // The waiter's command is Seq(3), still queued behind this batch.
        let early = engine
            .apply_batch([
                env(1, Command::Brightness(10)),
                env(2, Command::Brightness(20)),
            ])
            .expect("state changed");
        assert!(!early.has_applied(Seq(3)));

        let late = engine
            .apply_batch([Envelope::awaiting_reply(Seq(3), Command::Brightness(20))])
            .expect("awaited");
        assert!(late.has_applied(Seq(3)));
        assert_eq!(late.brightness, 20);
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
