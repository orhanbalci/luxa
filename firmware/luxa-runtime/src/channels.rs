//! The two seams between tasks.
//!
//! Commands flow one way (many producers → one consumer) and snapshots flow
//! the other (one producer → many observers). Those two shapes are the entire
//! concurrency design of slice 1, and keeping them here rather than threading
//! handles through constructors is what lets an ingress be added later without
//! touching the engine.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::watch::Watch;
use luxa_msg::{Command, Snapshot};

use crate::config::COMMAND_QUEUE_DEPTH;

/// Snapshot observers: the render task and the HTTP `/state` handler.
pub const SNAPSHOT_OBSERVERS: usize = 2;

/// Intent, flowing from every ingress into the engine.
///
/// A **queue**, because commands are events: dropping one loses a button press
/// the user actually made. Bounded, because unbounded queues on a device with
/// 400 KB of RAM only postpone the problem.
pub static COMMANDS: Channel<CriticalSectionRawMutex, Command, COMMAND_QUEUE_DEPTH> =
    Channel::new();

/// State, flowing from the engine out to everyone who renders or reports it.
///
/// A **watch**, not a queue, and the asymmetry is the point: a snapshot is not
/// an event, it is the current truth. A renderer that missed three snapshots
/// has lost nothing — it only ever wanted the newest. This is what stops a
/// slider drag from building a backlog the renderer has to chew through.
pub static SNAPSHOTS: Watch<CriticalSectionRawMutex, Snapshot, SNAPSHOT_OBSERVERS> = Watch::new();
