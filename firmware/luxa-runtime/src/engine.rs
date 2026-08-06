//! The engine task: drain the command queue, apply, publish once.
//!
//! Note how little is here. All the deciding happens in `luxa-core`, which has
//! no idea it is running under an executor; this task is the executor-shaped
//! wrapper around it, and that is deliberately all it is.

use luxa_core::Engine;

use crate::channels::{COMMANDS, SNAPSHOTS};

/// Runs the engine for the lifetime of the device.
#[embassy_executor::task]
pub async fn run() {
    let mut engine = Engine::new();
    let publisher = SNAPSHOTS.sender();

    // Publish the starting state so the render task has something to draw
    // before any command arrives — otherwise the strip stays dark until the
    // first HTTP request, which looks exactly like a bug.
    publisher.send(engine.snapshot());

    loop {
        // Block until there is something to do...
        let first = COMMANDS.receive().await;

        // ...then take everything else already queued behind it. A slider
        // drag arrives as a burst; this collapses the burst into one state
        // change and one publish instead of waking the renderer forty times.
        let batch =
            core::iter::once(first).chain(core::iter::from_fn(|| COMMANDS.try_receive().ok()));

        if let Some(snapshot) = engine.apply_batch(batch) {
            publisher.send(snapshot);
        }
    }
}
