//! Commands, where they came from, and their envelope.

use crate::{BoolOp, GlobalPatch, SegmentPatch, Seq, U8Op};

/// A request to change engine state, independent of how it arrived.
///
/// Commands describe *intent*, not effect, and they are deliberately small.
/// One control request that touches the fixture and several segments becomes
/// a *run* of commands — the fixture-wide changes first, then one per segment
/// — queued together and applied in one batch. Every command therefore stays
/// small enough to queue by value on a microcontroller, however many segments
/// a request touches, and the batch still produces a single publish.
///
/// `NAME` is the byte capacity of a segment name, matching the state's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command<const NAME: usize> {
    /// Fixture-wide changes: power, brightness, transitions.
    Global(GlobalPatch),
    /// Changes to one segment, or to every selected segment.
    Segment(SegmentPatch<NAME>),
}

impl<const NAME: usize> Command<NAME> {
    /// Switch the whole fixture on or off. Off remembers the brightness; on
    /// restores it.
    pub const fn power(on: bool) -> Self {
        Self::Global(GlobalPatch {
            on: Some(BoolOp::Set(on)),
            ..GlobalPatch::NONE
        })
    }

    /// Set the master brightness. `0` switches the fixture off; any other
    /// value switches it on at that level.
    pub const fn brightness(level: u8) -> Self {
        Self::Global(GlobalPatch {
            brightness: Some(U8Op::Set(level)),
            ..GlobalPatch::NONE
        })
    }
}

impl<const NAME: usize> From<GlobalPatch> for Command<NAME> {
    fn from(patch: GlobalPatch) -> Self {
        Self::Global(patch)
    }
}

impl<const NAME: usize> From<SegmentPatch<NAME>> for Command<NAME> {
    fn from(patch: SegmentPatch<NAME>) -> Self {
        Self::Segment(patch)
    }
}

/// Who caused a change.
///
/// The engine applies every command the same way. Origin matters for what
/// happens *around* a change — above all, not echoing a change back to the
/// peer it came from.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Origin {
    /// Restoring state at startup.
    Init,
    /// A user or client acting directly: a UI, an HTTP or WebSocket request.
    #[default]
    Direct,
    /// A physical button's built-in action.
    Button,
    /// A button or remote applying a stored preset or command.
    ButtonPreset,
    /// A state sync received from a peer fixture.
    Notification,
    /// Nightlight progress.
    Nightlight,
    /// A change that must not be announced to peers.
    NoNotify,
    /// A light-bridge integration.
    HueSync,
    /// A voice assistant.
    VoiceAssistant,
}

impl Origin {
    /// Whether a change from this origin should be announced to peers.
    pub const fn notifies_peers(self) -> bool {
        !matches!(self, Self::Notification | Self::NoNotify)
    }

    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

/// A set of [`Origin`]s — typically, where the changes in a batch came from.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Origins(u16);

impl Origins {
    /// No origins.
    pub const EMPTY: Self = Self(0);

    /// This set plus `origin`.
    #[must_use]
    pub const fn with(self, origin: Origin) -> Self {
        Self(self.0 | origin.bit())
    }

    /// Whether `origin` is in the set.
    pub const fn contains(self, origin: Origin) -> bool {
        self.0 & origin.bit() != 0
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether any origin in the set should be announced to peers.
    pub const fn notifies_peers(self) -> bool {
        let silent = Origin::Notification.bit() | Origin::NoNotify.bit();
        self.0 & !silent != 0
    }
}

/// A command as queued for the engine.
///
/// The envelope carries what the engine needs *around* a command without
/// knowing who sent it: its place in the input order, whether someone is
/// waiting to see the state it produces, and where it came from. It is still
/// not a reply channel — the reply is simply the next published state whose
/// [`applied_seq`](crate::State::applied_seq) has reached `seq`.
///
/// Numbering only matters to a sender that waits for its own result. Anyone
/// else can hand the engine a bare [`Command`]: it converts into an
/// *unnumbered*, [`Direct`](Origin::Direct) envelope, which is applied like any
/// other but leaves `applied_seq` where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Envelope<const NAME: usize> {
    /// Position in the engine's input order. Producers must hand these out in
    /// the same order they enqueue. [`Seq::ZERO`] marks an unnumbered command.
    pub seq: Seq,
    /// Whether the sender is waiting for the resulting state. The engine
    /// publishes for such a command even when it changed nothing, so the
    /// sender is never left waiting.
    pub awaits_reply: bool,
    /// Who caused the command.
    pub origin: Origin,
    /// The intent itself.
    pub command: Command<NAME>,
}

impl<const NAME: usize> Envelope<NAME> {
    /// A direct command whose sender does not wait for the result.
    pub const fn new(seq: Seq, command: Command<NAME>) -> Self {
        Self {
            seq,
            awaits_reply: false,
            origin: Origin::Direct,
            command,
        }
    }

    /// A direct command whose sender waits for the resulting state.
    pub const fn awaiting_reply(seq: Seq, command: Command<NAME>) -> Self {
        Self {
            awaits_reply: true,
            ..Self::new(seq, command)
        }
    }

    /// A direct command nobody waits for and nobody numbered.
    pub const fn unnumbered(command: Command<NAME>) -> Self {
        Self::new(Seq::ZERO, command)
    }

    /// The same envelope, attributed to `origin`.
    #[must_use]
    pub const fn with_origin(self, origin: Origin) -> Self {
        Self { origin, ..self }
    }
}

impl<const NAME: usize> From<Command<NAME>> for Envelope<NAME> {
    fn from(command: Command<NAME>) -> Self {
        Self::unnumbered(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SegmentTarget;

    #[test]
    fn convenience_constructors_are_global_patches() {
        assert_eq!(
            Command::<8>::power(false),
            Command::Global(GlobalPatch {
                on: Some(BoolOp::Set(false)),
                ..GlobalPatch::NONE
            })
        );
        assert_eq!(
            Command::<8>::brightness(42),
            Command::Global(GlobalPatch {
                brightness: Some(U8Op::Set(42)),
                ..GlobalPatch::NONE
            })
        );
    }

    #[test]
    fn patches_convert_into_commands() {
        let seg = SegmentPatch::<8>::for_selected();
        assert_eq!(Command::from(seg), Command::Segment(seg));
        assert_eq!(
            Command::<8>::from(GlobalPatch::NONE),
            Command::Global(GlobalPatch::NONE)
        );
    }

    #[test]
    fn a_bare_command_is_an_unnumbered_direct_envelope() {
        let e = Envelope::from(Command::<8>::power(true));
        assert_eq!(e.seq, Seq::ZERO);
        assert!(!e.awaits_reply);
        assert_eq!(e.origin, Origin::Direct);
    }

    #[test]
    fn origin_can_be_attributed() {
        let e = Envelope::awaiting_reply(Seq(3), Command::<8>::brightness(1))
            .with_origin(Origin::Button);
        assert_eq!(
            (e.seq, e.awaits_reply, e.origin),
            (Seq(3), true, Origin::Button)
        );
    }

    #[test]
    fn peer_notifications_and_silent_changes_are_not_announced() {
        assert!(Origin::Direct.notifies_peers());
        assert!(Origin::Button.notifies_peers());
        assert!(!Origin::Notification.notifies_peers());
        assert!(!Origin::NoNotify.notifies_peers());
    }

    #[test]
    fn origin_sets() {
        assert!(Origins::EMPTY.is_empty());
        assert!(!Origins::EMPTY.notifies_peers());

        let silent = Origins::EMPTY
            .with(Origin::Notification)
            .with(Origin::NoNotify);
        assert!(silent.contains(Origin::Notification));
        assert!(!silent.contains(Origin::Direct));
        assert!(!silent.notifies_peers());

        assert!(silent.with(Origin::Direct).notifies_peers());
    }

    /// The reason commands are granular: an envelope must stay small enough to
    /// queue dozens by value, even with WLED-sized 64-byte segment names.
    #[test]
    fn an_envelope_stays_small() {
        let size = core::mem::size_of::<Envelope<64>>();
        assert!(size <= 192, "Envelope<64> is {size} bytes");
        let _ = SegmentTarget::Selected;
    }
}
