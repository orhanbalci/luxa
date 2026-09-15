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
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use embassy_time::{Duration, Instant, Timer};
use luxa_msg::Seq;

use crate::config::{APPLY_TIMEOUT_MS, COMMAND_QUEUE_DEPTH, MAX_SEGMENTS, SEGMENT_NAME_LEN};

/// The fixture state at this board's capacities.
pub type State = luxa_msg::State<MAX_SEGMENTS, SEGMENT_NAME_LEN>;

/// A command at this board's capacities.
pub type Command = luxa_msg::Command<SEGMENT_NAME_LEN>;

/// A queued command at this board's capacities.
pub type Envelope = luxa_msg::Envelope<SEGMENT_NAME_LEN>;

/// Snapshot observers holding a receiver: the render task. Request handlers
/// read the latest snapshot without one.
pub const SNAPSHOT_OBSERVERS: usize = 1;

/// Intent, flowing from every ingress into the engine.
///
/// A **queue**, because commands are events: dropping one loses a button press
/// the user actually made. Bounded, because unbounded queues on a device with
/// 400 KB of RAM only postpone the problem.
///
/// Enqueue through [`submit_all`], never directly, so every command is
/// numbered.
pub static COMMANDS: Channel<CriticalSectionRawMutex, Envelope, COMMAND_QUEUE_DEPTH> =
    Channel::new();

/// State, flowing from the engine out to everyone who renders or reports it.
///
/// A **watch**, not a queue, and the asymmetry is the point: a state snapshot
/// is not an event, it is the current truth. A renderer that missed three has
/// lost nothing — it only ever wanted the newest. This is what stops a slider
/// drag from building a backlog the renderer has to chew through.
pub static SNAPSHOTS: Watch<CriticalSectionRawMutex, State, SNAPSHOT_OBSERVERS> = Watch::new();

/// Raised each time the engine publishes a state change clients should hear
/// about.
pub static STATE_CHANGES: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// The sequence number most recently handed out.
static LAST_SEQ: Mutex<CriticalSectionRawMutex, Cell<Seq>> = Mutex::new(Cell::new(Seq::ZERO));

/// The highest sequence number applied and published.
static APPLIED: Mutex<CriticalSectionRawMutex, Cell<Seq>> = Mutex::new(Cell::new(Seq::ZERO));

/// The sequence number applied by the latest publish that changed state.
static CHANGED: Mutex<CriticalSectionRawMutex, Cell<Seq>> = Mutex::new(Cell::new(Seq::ZERO));

/// The command queue could not take the whole request.
#[derive(Debug)]
pub struct QueueFull;

/// Numbers a request's commands and enqueues them together, without waiting.
///
/// `commands` is called twice — once to count, once to send — and must yield
/// the same commands both times. Either every command is queued or none is,
/// so a request is never half applied. The last command awaits a reply, so
/// the engine publishes once it is through even when nothing changed.
///
/// Numbering and enqueueing happen inside one critical section, so sequence
/// order is queue order however many producers race — the property the
/// engine's acknowledgement relies on. Returns the last number handed out, or
/// `None` for a request with no commands.
pub fn submit_all<I: Iterator<Item = Command>>(
    commands: impl Fn() -> I,
) -> Result<Option<Seq>, QueueFull> {
    let count = commands().count();
    if count == 0 {
        return Ok(None);
    }
    LAST_SEQ.lock(|last| {
        // Only the engine takes from the queue, and that only makes room, so
        // the capacity checked here is still there for every send below.
        if COMMANDS.free_capacity() < count {
            return Err(QueueFull);
        }
        let mut seq = last.get();
        for (index, command) in commands().enumerate() {
            seq = seq.next();
            let envelope = if index + 1 == count {
                Envelope::awaiting_reply(seq, command)
            } else {
                Envelope::new(seq, command)
            };
            let _ = COMMANDS.try_send(envelope);
        }
        last.set(seq);
        Ok(Some(seq))
    })
}

/// Records a publish: the engine has applied everything through `applied`,
/// and `changed` says whether clients should hear about it.
pub fn published(applied: Seq, changed: bool) {
    if changed {
        CHANGED.lock(|cell| cell.set(applied));
        STATE_CHANGES.signal(());
    }
    APPLIED.lock(|cell| cell.set(applied));
}

/// Waits until the engine has published the state after command `seq`.
/// Returns `false` if that took longer than [`APPLY_TIMEOUT_MS`].
pub async fn applied(seq: Seq) -> bool {
    let deadline = Instant::now() + Duration::from_millis(APPLY_TIMEOUT_MS);
    while APPLIED.lock(Cell::get) < seq {
        if Instant::now() >= deadline {
            return false;
        }
        Timer::after_millis(2).await;
    }
    true
}

/// Whether state changed in a publish that applied command `seq` or later.
pub fn changed_since(seq: Seq) -> bool {
    CHANGED.lock(Cell::get) >= seq
}
