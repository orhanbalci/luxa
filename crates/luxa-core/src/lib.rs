//! The engine — the single writer of application state.
//!
//! Every ingress (HTTP today; MQTT, buttons and schedules later) produces
//! [`Command`]s onto one channel. This crate drains that channel, applies the
//! commands, and publishes a [`Snapshot`]. Nothing else in Luxa mutates state,
//! which is what makes "what is the fixture doing right now?" a question with
//! exactly one answer.
//!
//! It knows nothing about sockets, executors or pixels. There is no `async`
//! here and no channel type — the runtime owns those and calls in. That is why
//! every test below is a plain synchronous test with no executor to spin up.
//!
//! # Drain, then publish
//!
//! [`Engine::apply_batch`] is the real loop, not a slice-1 shortcut. The
//! runtime drains everything currently queued, applies it, and publishes *one*
//! snapshot for the batch. This matters as soon as there is a slider in the
//! UI: dragging it emits commands far faster than frames render, and
//! publishing per command would make the renderer chase a backlog it can never
//! catch up with. Coalescing means a burst of forty brightness commands costs
//! one snapshot and the renderer always sees the newest value.
//!
//! ```
//! use luxa_core::Engine;
//! use luxa_msg::Command;
//!
//! let mut engine = Engine::new();
//! let published = engine.apply_batch([
//!     Command::Brightness(10),
//!     Command::Brightness(20),
//!     Command::Brightness(30),
//! ]);
//!
//! // Three commands in, one snapshot out, carrying only the final value.
//! assert_eq!(published.unwrap().brightness, 30);
//! ```

#![no_std]
#![forbid(unsafe_code)]

use luxa_msg::{Command, Snapshot};

/// Everything the fixture currently is.
///
/// Only [`Engine`] can mutate this; the rest of the system observes it through
/// an immutable borrow or, more usually, a published [`Snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppState {
    /// Whether the fixture is on.
    pub power: bool,
    /// Global brightness, `0`–`255`.
    pub brightness: u8,
}

impl AppState {
    /// The state a fresh engine starts in.
    pub const DEFAULT: Self = Self {
        power: Snapshot::DEFAULT.power,
        brightness: Snapshot::DEFAULT.brightness,
    };
}

impl Default for AppState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Owns [`AppState`] and is the only thing that writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Engine {
    state: AppState,
}

impl Engine {
    /// An engine in the default state.
    pub const fn new() -> Self {
        Self {
            state: AppState::DEFAULT,
        }
    }

    /// An engine restored to a known state — from persistence, in a later slice.
    pub const fn with_state(state: AppState) -> Self {
        Self { state }
    }

    /// The current state, read-only.
    pub const fn state(&self) -> &AppState {
        &self.state
    }

    /// The current state as a publishable snapshot.
    ///
    /// The runtime publishes this once at startup so the render task has
    /// something to draw before any command arrives.
    pub const fn snapshot(&self) -> Snapshot {
        Snapshot {
            power: self.state.power,
            brightness: self.state.brightness,
        }
    }

    /// Applies one command, returning whether it actually changed anything.
    ///
    /// Idempotent commands report `false` so a batch of no-ops can skip its
    /// publish entirely.
    pub fn apply(&mut self, command: Command) -> bool {
        let before = self.state;
        match command {
            Command::Power(on) => self.state.power = on,
            Command::Brightness(level) => self.state.brightness = level,
        }
        self.state != before
    }

    /// Applies a whole batch and returns the snapshot to publish, if any.
    ///
    /// Returns `None` when nothing changed — there is no point waking the
    /// render path to tell it the world is exactly as it left it.
    pub fn apply_batch(&mut self, commands: impl IntoIterator<Item = Command>) -> Option<Snapshot> {
        let mut changed = false;
        for command in commands {
            // Not `||`, which would short-circuit and stop applying.
            changed |= self.apply(command);
        }
        changed.then(|| self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_the_default_state() {
        assert_eq!(Engine::new().snapshot(), Snapshot::DEFAULT);
    }

    #[test]
    fn applies_power_and_brightness() {
        let mut engine = Engine::new();
        assert!(engine.apply(Command::Power(false)));
        assert!(engine.apply(Command::Brightness(42)));
        assert_eq!(
            engine.snapshot(),
            Snapshot {
                power: false,
                brightness: 42
            }
        );
    }

    #[test]
    fn power_and_brightness_are_independent() {
        let mut engine = Engine::new();
        engine.apply(Command::Brightness(200));
        engine.apply(Command::Power(false));
        // Turning off must not forget the brightness the user chose — turning
        // back on has to restore it, not reset to a default.
        assert_eq!(engine.state().brightness, 200);
        engine.apply(Command::Power(true));
        assert_eq!(
            engine.snapshot(),
            Snapshot {
                power: true,
                brightness: 200
            }
        );
    }

    #[test]
    fn a_redundant_command_reports_no_change() {
        let mut engine = Engine::new();
        assert!(engine.apply(Command::Brightness(7)));
        assert!(!engine.apply(Command::Brightness(7)));
    }

    #[test]
    fn a_batch_publishes_once_with_the_final_value() {
        let mut engine = Engine::new();
        let published = engine
            .apply_batch([
                Command::Brightness(10),
                Command::Brightness(20),
                Command::Power(false),
                Command::Brightness(30),
            ])
            .expect("state changed, so a snapshot is due");
        assert_eq!(
            published,
            Snapshot {
                power: false,
                brightness: 30
            }
        );
        assert_eq!(published, engine.snapshot());
    }

    #[test]
    fn an_empty_batch_publishes_nothing() {
        assert_eq!(Engine::new().apply_batch([]), None);
    }

    #[test]
    fn a_batch_of_no_ops_publishes_nothing() {
        let mut engine = Engine::new();
        let current = engine.snapshot();
        assert_eq!(
            engine.apply_batch([
                Command::Power(current.power),
                Command::Brightness(current.brightness),
            ]),
            None
        );
    }

    #[test]
    fn a_batch_that_nets_out_to_no_change_still_publishes() {
        // Intermediate states are not observable, but the engine must not try
        // to be clever about it: `changed` tracks whether any step moved, and
        // reporting a redundant snapshot is far safer than dropping a real one.
        let mut engine = Engine::new();
        let before = engine.snapshot();
        let published = engine.apply_batch([Command::Brightness(1), Command::Brightness(128)]);
        assert_eq!(published, Some(before));
    }

    #[test]
    fn every_command_in_a_batch_is_applied() {
        // Regression guard: a `||` here instead of `|=` would short-circuit
        // after the first change and silently drop the rest of the batch.
        let mut engine = Engine::new();
        engine.apply_batch([Command::Power(false), Command::Brightness(3)]);
        assert_eq!(
            engine.snapshot(),
            Snapshot {
                power: false,
                brightness: 3
            }
        );
    }

    #[test]
    fn state_can_be_restored() {
        let state = AppState {
            power: false,
            brightness: 9,
        };
        assert_eq!(Engine::with_state(state).state(), &state);
    }
}
