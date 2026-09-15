//! The two seams between tasks.
//!
//! Commands flow one way (many producers → one consumer) and state flows the
//! other (one producer → many observers). Those two shapes are the entire
//! concurrency design, and keeping them here rather than threading handles
//! through constructors is what lets an ingress be added later without
//! touching the engine.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::watch::Watch;
use luxa_msg::{Command, Envelope, Seq};

use crate::config::{COMMAND_QUEUE_DEPTH, MAX_SEGMENTS, SEGMENT_NAME_LEN};

/// The fixture state at this board's capacities.
pub type State = luxa_msg::State<MAX_SEGMENTS, SEGMENT_NAME_LEN>;

/// Snapshot observers: the render task and the HTTP `/state` handler.
pub const SNAPSHOT_OBSERVERS: usize = 2;

/// Intent, flowing from every ingress into the engine.
///
/// A **queue**, because commands are events: dropping one loses a button press
/// the user actually made. Bounded, because unbounded queues on a device with
/// 400 KB of RAM only postpone the problem.
///
/// Enqueue through [`submit`], never directly, so every command is numbered.
pub static COMMANDS: Channel<CriticalSectionRawMutex, Envelope, COMMAND_QUEUE_DEPTH> =
    Channel::new();

/// State, flowing from the engine out to everyone who renders or reports it.
///
/// A **watch**, not a queue, and the asymmetry is the point: a state snapshot
/// is not an event, it is the current truth. A renderer that missed three has
/// lost nothing — it only ever wanted the newest. This is what stops a slider
/// drag from building a backlog the renderer has to chew through.
pub static SNAPSHOTS: Watch<CriticalSectionRawMutex, State, SNAPSHOT_OBSERVERS> = Watch::new();

/// The sequence number most recently handed out.
static LAST_SEQ: Mutex<CriticalSectionRawMutex, Cell<Seq>> = Mutex::new(Cell::new(Seq::ZERO));

/// The command queue was full.
#[derive(Debug)]
pub struct QueueFull;

/// Numbers a command and enqueues it, without waiting.
///
/// Numbering and enqueueing happen inside one critical section, so sequence
/// order is queue order however many producers race — the property the
/// engine's acknowledgement relies on. A command that does not fit is not
/// numbered, so the sequence has no gaps a waiter could stall on.
pub fn submit(command: Command) -> Result<Seq, QueueFull> {
    LAST_SEQ.lock(|last| {
        let seq = last.get().next();
        match COMMANDS.try_send(Envelope::new(seq, command)) {
            Ok(()) => {
                last.set(seq);
                Ok(seq)
            }
            Err(TrySendError::Full(_)) => Err(QueueFull),
        }
    })
}
