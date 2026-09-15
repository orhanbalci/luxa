//! Commands and their envelope.

use crate::Seq;

/// A request to change engine state, independent of how it arrived.
///
/// Commands describe *intent*, not effect: `Brightness(128)` means "the user
/// asked for half brightness", and what that does to pixels is decided
/// downstream. Nothing here is transport-shaped — no socket, no request object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Turn the whole fixture on or off. Off remembers the brightness; on
    /// restores it.
    Power(bool),
    /// Set the master brightness, `0`–`255`. `0` switches the fixture off; any
    /// other value switches it on at that level.
    Brightness(u8),
}

/// A command as queued for the engine.
///
/// The envelope carries what the engine needs to *acknowledge* a command
/// without knowing who sent it: its place in the input order, and whether
/// someone is waiting to see the state it produces. It is still not a reply
/// channel — the reply is simply the next published state whose
/// [`applied_seq`](crate::State::applied_seq) has reached `seq`.
///
/// Numbering only matters to a sender that waits for its own result. Anyone
/// else can hand the engine a bare [`Command`]: it converts into an
/// *unnumbered* envelope, which is applied like any other but leaves
/// `applied_seq` where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Envelope {
    /// Position in the engine's input order. Producers must hand these out in
    /// the same order they enqueue. [`Seq::ZERO`] marks an unnumbered command.
    pub seq: Seq,
    /// Whether the sender is waiting for the resulting state. The engine
    /// publishes for such a command even when it changed nothing, so the
    /// sender is never left waiting.
    pub awaits_reply: bool,
    /// The intent itself.
    pub command: Command,
}

impl Envelope {
    /// A command whose sender does not wait for the result.
    pub const fn new(seq: Seq, command: Command) -> Self {
        Self {
            seq,
            awaits_reply: false,
            command,
        }
    }

    /// A command whose sender waits for the resulting state.
    pub const fn awaiting_reply(seq: Seq, command: Command) -> Self {
        Self {
            seq,
            awaits_reply: true,
            command,
        }
    }

    /// A command nobody waits for and nobody numbered.
    pub const fn unnumbered(command: Command) -> Self {
        Self::new(Seq::ZERO, command)
    }
}

impl From<Command> for Envelope {
    fn from(command: Command) -> Self {
        Self::unnumbered(command)
    }
}
