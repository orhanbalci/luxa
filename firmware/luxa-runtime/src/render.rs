//! The render task: the frame pipeline, on a fixed ticker.
//!
//! Read the loop below and note what it does *not* do. It does not know what a
//! rainbow is, how brightness scales a channel, or that WS2812 wants green
//! first. Every one of those lives in a portable crate that was tested on a
//! laptop; this task's whole contribution is calling them in order, at the
//! right rate, with the right clock.
//!
//! If a line ever appears here that multiplies a pixel, picks a hue, or
//! reorders a channel, it is in the wrong crate.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Receiver;
use embassy_time::{Duration, Instant, Ticker};
use luxa_canvas::Canvas;
use luxa_driver_esp_rmt::{RmtWs2812, codes_for};
use luxa_effect::Ctx;
use luxa_msg::Snapshot;
use luxa_segment::Compositor;
use luxa_wire::Ws2812;

use crate::channels::SNAPSHOT_OBSERVERS;
use crate::config::{FRAME_MS, LEDS, PROFILE};

/// The strip driver, sized for this board's canvas.
pub type Strip = RmtWs2812<'static, { codes_for(LEDS) }>;

/// Snapshot observer handed to the render task.
pub type Snapshots = Receiver<'static, CriticalSectionRawMutex, Snapshot, SNAPSHOT_OBSERVERS>;

/// Renders frames forever.
#[embassy_executor::task]
pub async fn run(mut strip: Strip, mut snapshots: Snapshots) {
    let mut canvas = Canvas::<LEDS>::black();
    let mut compositor = Compositor::default();
    let chipset = Ws2812::new(PROFILE.color_order);
    let mut wire = [0u8; Ws2812::buffer_len(LEDS)];

    // The engine publishes its initial state at startup, so this wait is
    // bounded and saves drawing one frame of guessed state.
    let mut snapshot = snapshots.changed().await;

    let mut ticker = Ticker::every(Duration::from_millis(FRAME_MS));

    loop {
        ticker.next().await;

        // Take the newest state if there is one, otherwise keep drawing the
        // last. Never block here: a frame is due whether or not anything moved.
        if let Some(latest) = snapshots.try_changed() {
            snapshot = latest;
        }

        // The one place the platform clock is narrowed to the animation clock.
        // Every effect in this frame is handed this same value, so they cannot
        // drift apart by however long the frame took to render.
        let ctx = Ctx::from_micros_u64(Instant::now().as_micros());

        compositor.render(canvas.as_mut_slice(), &ctx);
        luxa_output::apply(canvas.as_mut_slice(), &snapshot);

        // The strip may be shorter than the canvas; send only what exists.
        let visible = &canvas.as_slice()[..PROFILE.pixel_count.min(LEDS)];

        // Both arms swallow their error on purpose. `wire` is sized from the
        // same constant `encode` measures against so the encode cannot fail,
        // and a failed write is one dropped frame with the next 16 ms away —
        // panicking in the render loop would take the whole demo down over
        // something the next iteration fixes by itself.
        if let Ok(len) = chipset.encode(visible, &mut wire) {
            let _ = strip.write(&wire[..len]).await;
        }
    }
}
