//! The whole portable render pipeline, exercised on a laptop.
//!
//! `engine → state → compositor → canvas → output → wire → bytes`
//!
//! Every crate in the chain below is `no_std`, allocation-free and free of any
//! HAL. That is the claim this file exists to prove: the only thing the
//! firmware still needs from real (or simulated) silicon is somebody to clock
//! these bytes out of a pin. If the strip is wrong, it is wrong *there* — the
//! rest is pinned here.
//!
//! It lives in `luxa-segment` because the compositor is the head of the render
//! chain; the dev-dependencies below are deliberately one-directional
//! (segment never depends on output or wire at build time).

use luxa_canvas::Canvas;
use luxa_color::{ColorOrder, Crgb};
use luxa_core::Engine;
use luxa_effect::effects::Rainbow;
use luxa_effect::{Ctx, EffectKind};
use luxa_msg::{Command, Layout, SegmentPatch, State, U8Op};
use luxa_segment::{CATALOGUE, Compositor};
use luxa_wire::Ws2812;

const LEDS: usize = 60;
const LAYOUT: Layout = Layout::new(LEDS as u16);
const RAINBOW: u8 = EffectKind::Rainbow(Rainbow::new()).id();

type TestState = State<4, 16>;
type TestEngine = Engine<4, 16>;
type Wire = [u8; Ws2812::buffer_len(LEDS)];

/// One frame of the real pipeline, exactly as the render task runs it.
fn frame(compositor: &mut Compositor<4, LEDS>, state: &TestState, now_ms: u32) -> Wire {
    let mut canvas = Canvas::<LEDS>::black();

    // The clock is narrowed once, here, and handed down. Nothing below reads
    // a clock of its own.
    compositor.render(canvas.as_mut_slice(), state, &Ctx::from_millis(now_ms));
    luxa_output::apply_brightness(canvas.as_mut_slice(), state.brightness);

    let mut wire = [0u8; Ws2812::buffer_len(LEDS)];
    let n = Ws2812::new(ColorOrder::Grb)
        .encode(canvas.as_slice(), &mut wire)
        .expect("buffer is sized from the same constant");
    assert_eq!(n, wire.len());
    wire
}

fn engine() -> TestEngine {
    TestEngine::new(LAYOUT, CATALOGUE)
}

fn effect(id: u8, patch: SegmentPatch<16>) -> Command<16> {
    Command::Segment(SegmentPatch {
        effect: Some(U8Op::Set(id)),
        ..patch
    })
}

fn rainbow_state() -> TestState {
    let mut e = engine();
    e.apply(effect(RAINBOW, SegmentPatch::for_id(0)));
    e.state().clone()
}

fn pixels(wire: &Wire) -> impl Iterator<Item = &[u8]> {
    wire.chunks(3)
}

#[test]
fn a_default_frame_lights_the_strip_in_one_colour() {
    let bytes = frame(&mut Compositor::new(), &TestState::new(LAYOUT), 0);
    let first = pixels(&bytes).next().unwrap();
    assert!(
        first.iter().any(|b| *b != 0),
        "the default state must produce visible light — 'first light' in byte form"
    );
    assert!(
        pixels(&bytes).all(|p| p == first),
        "a fresh fixture runs a solid colour across every LED"
    );
}

#[test]
fn power_off_produces_an_all_zero_frame() {
    let mut state = rainbow_state();
    state.brightness = 0;
    let bytes = frame(&mut Compositor::new(), &state, 1234);
    assert!(
        bytes.iter().all(|b| *b == 0),
        "power off must reach the wire as literal zeros"
    );
}

#[test]
fn brightness_scales_the_whole_frame() {
    let mut compositor = Compositor::new();
    let mut state = rainbow_state();
    state.brightness = 255;
    let full = frame(&mut compositor, &state, 1234);
    state.brightness = 64;
    let dim = frame(&mut compositor, &state, 1234);

    let sum = |b: &[u8]| b.iter().map(|x| *x as u32).sum::<u32>();
    assert!(sum(&dim) < sum(&full));
    // Every byte must be dimmed or already dark — never brightened.
    for (f, d) in full.iter().zip(dim.iter()) {
        assert!(d <= f, "brightness must only ever attenuate");
    }
}

#[test]
fn the_animation_advances_with_the_clock() {
    let mut compositor = Compositor::new();
    let state = rainbow_state();
    assert_ne!(
        frame(&mut compositor, &state, 0),
        frame(&mut compositor, &state, 512),
        "the strip must actually animate"
    );
}

#[test]
fn a_frame_is_reproducible_from_its_timestamp_alone() {
    // The payoff of the Ctx decision: given the same clock value and state,
    // the bytes on the wire are identical no matter what ran before.
    let state = rainbow_state();
    let baseline = frame(&mut Compositor::new(), &state, 9_000);

    let mut warmed = Compositor::new();
    for t in [0, 17, 4_321, 100_000] {
        frame(&mut warmed, &state, t);
    }
    assert_eq!(frame(&mut warmed, &state, 9_000), baseline);
}

#[test]
fn commands_reach_the_wire() {
    // The full control path, minus the socket: a request becomes a Command,
    // the engine folds it into State, the render path obeys it.
    let mut engine = engine();
    let mut compositor = Compositor::new();

    let lit = engine
        .apply_batch([Command::power(true), Command::brightness(255)])
        .expect("state changed")
        .state
        .clone();
    assert!(frame(&mut compositor, &lit, 500).iter().any(|b| *b != 0));

    let off = engine
        .apply_batch([Command::power(false)])
        .expect("state changed")
        .state
        .clone();
    assert!(frame(&mut compositor, &off, 500).iter().all(|b| *b == 0));

    // ...and back on, at the brightness the user had chosen before.
    let on_again = engine
        .apply_batch([Command::power(true)])
        .expect("state changed")
        .state
        .clone();
    assert_eq!(on_again.brightness, 255);
    assert!(
        frame(&mut compositor, &on_again, 500)
            .iter()
            .any(|b| *b != 0),
        "switching back on must restore the previous brightness"
    );
}

#[test]
fn two_segments_render_their_own_effects_into_their_own_ranges() {
    let mut engine = engine();
    let state = engine
        .apply_batch([
            // Segment 0 keeps its solid colour but shrinks to the first half...
            Command::Segment(SegmentPatch {
                stop: Some(30),
                ..SegmentPatch::for_id(0)
            }),
            // ...and a new segment runs the rainbow over the second half.
            effect(
                RAINBOW,
                SegmentPatch {
                    start: Some(30),
                    stop: Some(60),
                    ..SegmentPatch::for_id(1)
                },
            ),
        ])
        .expect("state changed")
        .state
        .clone();

    let bytes = frame(&mut Compositor::new(), &state, 777);
    let (solid, rainbow) = bytes.split_at(30 * 3);

    let first = &solid[..3];
    assert!(first.iter().any(|b| *b != 0));
    assert!(
        solid.chunks(3).all(|p| p == first),
        "the solid segment fills its range with one colour"
    );
    assert!(
        rainbow.chunks(3).any(|p| p != &rainbow[..3]),
        "the rainbow segment varies along its range"
    );
    assert!(
        rainbow.chunks(3).all(|p| p != first),
        "the rainbow does not spill the solid colour into its range"
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
