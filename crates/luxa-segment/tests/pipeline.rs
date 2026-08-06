//! The whole portable render pipeline, exercised on a laptop.
//!
//! `engine → snapshot → compositor → canvas → output → wire → bytes`
//!
//! Every crate in the chain below is `no_std`, allocation-free and free of any
//! HAL. That is the claim this file exists to prove: the only thing slice 1
//! still needs from real (or simulated) silicon is somebody to clock these
//! bytes out of a pin. If the strip is wrong, it is wrong *there* — the rest
//! is pinned here.
//!
//! It lives in `luxa-segment` because the compositor is the head of the render
//! chain; the dev-dependencies below are deliberately one-directional
//! (segment never depends on output or wire at build time).

use luxa_canvas::Canvas;
use luxa_color::{ColorOrder, Crgb};
use luxa_core::Engine;
use luxa_effect::Ctx;
use luxa_msg::{Command, Snapshot};
use luxa_segment::Compositor;
use luxa_wire::Ws2812;

const LEDS: usize = 60;

/// One frame of the real pipeline, exactly as the render task will run it.
fn frame(
    compositor: &mut Compositor,
    snapshot: &Snapshot,
    now_ms: u32,
) -> [u8; Ws2812::buffer_len(LEDS)] {
    let mut canvas = Canvas::<LEDS>::black();

    // The clock is narrowed once, here, and handed down. Nothing below reads
    // a clock of its own.
    compositor.render(canvas.as_mut_slice(), &Ctx::from_millis(now_ms));
    luxa_output::apply(canvas.as_mut_slice(), snapshot);

    let mut wire = [0u8; Ws2812::buffer_len(LEDS)];
    let n = Ws2812::new(ColorOrder::Grb)
        .encode(canvas.as_slice(), &mut wire)
        .expect("buffer is sized from the same constant");
    assert_eq!(n, wire.len());
    wire
}

#[test]
fn a_default_frame_lights_the_strip() {
    let bytes = frame(&mut Compositor::default(), &Snapshot::DEFAULT, 0);
    assert!(
        bytes.iter().any(|b| *b != 0),
        "the default state must produce visible light — this is 'first light' in byte form"
    );
}

#[test]
fn power_off_produces_an_all_zero_frame() {
    let snapshot = Snapshot {
        power: false,
        brightness: 255,
    };
    let bytes = frame(&mut Compositor::default(), &snapshot, 1234);
    assert!(
        bytes.iter().all(|b| *b == 0),
        "power off must reach the wire as literal zeros"
    );
}

#[test]
fn brightness_scales_the_whole_frame() {
    let mut compositor = Compositor::default();
    let full = frame(
        &mut compositor,
        &Snapshot {
            power: true,
            brightness: 255,
        },
        1234,
    );
    let dim = frame(
        &mut compositor,
        &Snapshot {
            power: true,
            brightness: 64,
        },
        1234,
    );

    let sum = |b: &[u8]| b.iter().map(|x| *x as u32).sum::<u32>();
    assert!(sum(&dim) < sum(&full));
    // Every byte must be dimmed or already dark — never brightened.
    for (f, d) in full.iter().zip(dim.iter()) {
        assert!(d <= f, "brightness must only ever attenuate");
    }
}

#[test]
fn the_animation_advances_with_the_clock() {
    let mut compositor = Compositor::default();
    let a = frame(&mut compositor, &Snapshot::DEFAULT, 0);
    let b = frame(&mut compositor, &Snapshot::DEFAULT, 512);
    assert_ne!(a, b, "the strip must actually animate");
}

#[test]
fn a_frame_is_reproducible_from_its_timestamp_alone() {
    // The payoff of the Ctx decision: given the same clock value and snapshot,
    // the bytes on the wire are identical no matter what ran before.
    let baseline = frame(&mut Compositor::default(), &Snapshot::DEFAULT, 9_000);

    let mut warmed = Compositor::default();
    for t in [0, 17, 4_321, 100_000] {
        frame(&mut warmed, &Snapshot::DEFAULT, t);
    }
    assert_eq!(frame(&mut warmed, &Snapshot::DEFAULT, 9_000), baseline);
}

#[test]
fn commands_reach_the_wire() {
    // The full slice-1 control path, minus the socket: a POST becomes a
    // Command, the engine folds it into a Snapshot, the render path obeys it.
    let mut engine = Engine::new();
    let mut compositor = Compositor::default();

    let lit = engine
        .apply_batch([Command::Power(true), Command::Brightness(255)])
        .expect("state changed");
    assert!(frame(&mut compositor, &lit, 500).iter().any(|b| *b != 0));

    let off = engine
        .apply_batch([Command::Power(false)])
        .expect("state changed");
    assert!(frame(&mut compositor, &off, 500).iter().all(|b| *b == 0));

    // ...and back on, at the brightness the user had chosen before.
    let on_again = engine
        .apply_batch([Command::Power(true)])
        .expect("state changed");
    assert_eq!(on_again.brightness, 255);
    assert!(
        frame(&mut compositor, &on_again, 500)
            .iter()
            .any(|b| *b != 0),
        "toggling back on must restore the previous brightness"
    );
}

#[test]
fn the_first_pixel_is_first_on_the_wire_in_grb() {
    // Pin the seam between the canvas and the strip: canvas index 0 is the
    // pixel nearest the controller, and its green channel leads.
    let mut canvas = Canvas::<2>::black();
    canvas[0] = Crgb::new(0x10, 0x20, 0x30);
    canvas[1] = Crgb::new(0x40, 0x50, 0x60);

    let mut wire = [0u8; Ws2812::buffer_len(2)];
    Ws2812::new(ColorOrder::Grb)
        .encode(canvas.as_slice(), &mut wire)
        .unwrap();

    assert_eq!(wire, [0x20, 0x10, 0x30, 0x50, 0x40, 0x60]);
}
