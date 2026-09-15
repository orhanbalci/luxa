//! The compositor: which effect renders into which pixels.
//!
//! This crate answers one question — given the fixture's [`State`] and a
//! canvas, who draws where. Each active segment runs its own effect over its
//! own range of LEDs, with the segment's settings applied around it: mirroring,
//! reversing and opacity. Brightness is not applied here; that is the output
//! stage's job, once, over the finished frame.
//!
//! It is also where effects meet the engine: [`CATALOGUE`] tells the engine
//! exactly which effect and palette ids the renderer can draw.
//!
//! ```
//! use luxa_color::Crgb;
//! use luxa_effect::Ctx;
//! use luxa_msg::{Layout, State};
//! use luxa_segment::Compositor;
//!
//! // A freshly booted fixture: one segment, solid warm orange.
//! let state = State::<4, 16>::new(Layout::new(8));
//! let mut canvas = [Crgb::new(0, 0, 0); 8];
//! Compositor::<4>::new().render(&mut canvas, &state, &Ctx::from_millis(0));
//! assert!(canvas.iter().all(|p| *p == Crgb::new(255, 160, 0)));
//! ```

#![no_std]
#![forbid(unsafe_code)]

use luxa_color::{Crgb, nscale8};
use luxa_effect::{Ctx, Effect, EffectKind, PALETTES, Params};
use luxa_msg::{Catalogue, IdSet, Segment, State};

/// The effect and palette ids the renderer can draw: every effect in
/// [`EffectKind::ALL`] and every palette in [`PALETTES`].
///
/// Hand this to the engine so it accepts exactly the selections that will
/// actually appear.
pub const CATALOGUE: Catalogue = {
    let mut effects = IdSet::EMPTY;
    let mut i = 0;
    while i < EffectKind::ALL.len() {
        effects = effects.with(EffectKind::ALL[i].id());
        i += 1;
    }
    let mut palettes = IdSet::EMPTY;
    let mut j = 0;
    while j < PALETTES.len() {
        palettes = palettes.with(PALETTES[j].id);
        j += 1;
    }
    Catalogue::new(effects, palettes)
};

/// What a segment's effect instance was created for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Setup {
    effect: u8,
    drawn: usize,
}

/// One segment's running effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    effect: EffectKind,
    setup: Option<Setup>,
}

impl Slot {
    const EMPTY: Self = Self {
        effect: EffectKind::Solid(luxa_effect::effects::Solid::new()),
        setup: None,
    };
}

/// Renders every active segment of a [`State`] into a canvas.
///
/// Each segment slot keeps its own effect instance, so stateful effects animate
/// independently. An instance starts fresh when its segment switches effect or
/// changes how many pixels the effect draws.
///
/// `SEGMENTS` matches the state's segment capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compositor<const SEGMENTS: usize> {
    slots: [Slot; SEGMENTS],
}

impl<const SEGMENTS: usize> Compositor<SEGMENTS> {
    /// A compositor with no effects running yet.
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; SEGMENTS],
        }
    }

    /// Renders one frame of `state` into `canvas`.
    ///
    /// Pixels no active segment covers are black. Where segments overlap, the
    /// later one draws on top; a segment that is off leaves what lies beneath.
    /// Segments reaching past the canvas are clipped to it.
    pub fn render<const NAME: usize>(
        &mut self,
        canvas: &mut [Crgb],
        state: &State<SEGMENTS, NAME>,
        ctx: &Ctx,
    ) {
        canvas.fill(Crgb::new(0, 0, 0));
        for (id, segment) in state.active_segments() {
            render_segment(&mut self.slots[id], canvas, segment, ctx);
        }
    }
}

impl<const SEGMENTS: usize> Default for Compositor<SEGMENTS> {
    fn default() -> Self {
        Self::new()
    }
}

fn render_segment<const NAME: usize>(
    slot: &mut Slot,
    canvas: &mut [Crgb],
    segment: &Segment<NAME>,
    ctx: &Ctx,
) {
    if !segment.on || segment.opacity == 0 {
        return;
    }
    let end = usize::from(segment.stop).min(canvas.len());
    let start = usize::from(segment.start).min(end);
    let view = &mut canvas[start..end];
    let len = view.len();
    if len == 0 {
        return;
    }

    // A mirrored segment's effect draws the first half, which is reflected.
    let drawn = if segment.mirror { len.div_ceil(2) } else { len };
    let setup = Setup {
        effect: segment.effect.0,
        drawn,
    };
    if slot.setup != Some(setup) {
        slot.effect = EffectKind::from_id(setup.effect).unwrap_or_default();
        slot.setup = Some(setup);
    }

    let params = Params {
        speed: segment.speed,
        intensity: segment.intensity,
        colors: segment.colors,
    };
    let half = &mut view[..drawn];
    slot.effect.render(half, ctx, &params);
    if segment.reverse {
        half.reverse();
    }
    if segment.mirror {
        for i in 0..len / 2 {
            view[len - 1 - i] = view[i];
        }
    }
    // Opacity fades the segment towards black. Blending overlapping segments
    // into each other arrives with blend modes.
    if segment.opacity < u8::MAX {
        nscale8(view, segment.opacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_effect::effects::Rainbow;
    use luxa_msg::{EffectId, Layout, Rgbw};

    type TestState = State<4, 8>;

    const BLACK: Crgb = Crgb::new(0, 0, 0);
    const RED: Rgbw = Rgbw::new(255, 0, 0, 0);
    const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);
    const RAINBOW: EffectId = EffectId(EffectKind::Rainbow(Rainbow::new()).id());

    fn solid(start: u16, stop: u16, color: Rgbw) -> Segment<8> {
        let mut s = Segment::new(start, stop);
        s.colors[0] = color;
        s
    }

    fn rainbow(start: u16, stop: u16) -> Segment<8> {
        let mut s = Segment::new(start, stop);
        s.effect = RAINBOW;
        s
    }

    fn state(segments: &[Segment<8>]) -> TestState {
        let mut st = TestState::new(Layout::new(8));
        st.segments_mut()[0] = segments[0];
        for s in &segments[1..] {
            st.push_segment(*s).unwrap();
        }
        st
    }

    fn draw(st: &TestState) -> [Crgb; 8] {
        let mut canvas = [Crgb::new(1, 1, 1); 8];
        Compositor::<4>::new().render(&mut canvas, st, &Ctx::from_millis(321));
        canvas
    }

    /// The rainbow as its effect draws it over `len` pixels, unmodified.
    fn raw_rainbow(len: usize) -> [Crgb; 8] {
        let mut buf = [BLACK; 8];
        Rainbow.render(&mut buf[..len], &Ctx::from_millis(321), &Params::DEFAULT);
        buf
    }

    #[test]
    fn pixels_outside_every_segment_are_black() {
        let out = draw(&state(&[solid(0, 4, RED)]));
        assert!(out[..4].iter().all(|p| *p == RED.rgb()));
        assert!(out[4..].iter().all(|p| *p == BLACK));
    }

    #[test]
    fn each_segment_runs_its_own_effect() {
        let out = draw(&state(&[solid(0, 4, RED), rainbow(4, 8)]));
        assert!(out[..4].iter().all(|p| *p == RED.rgb()));
        assert_eq!(out[4..], raw_rainbow(4)[..4]);
    }

    #[test]
    fn later_segments_draw_on_top() {
        let out = draw(&state(&[solid(0, 8, RED), solid(2, 4, BLUE)]));
        assert_eq!(out[1], RED.rgb());
        assert_eq!(out[2..4], [BLUE.rgb(); 2]);
        assert_eq!(out[4], RED.rgb());
    }

    #[test]
    fn a_segment_that_is_off_leaves_what_lies_beneath() {
        let mut top = solid(2, 4, BLUE);
        top.on = false;
        let out = draw(&state(&[solid(0, 8, RED), top]));
        assert!(out.iter().all(|p| *p == RED.rgb()));
    }

    #[test]
    fn deleted_segments_are_not_drawn() {
        let mut st = state(&[solid(0, 4, RED), solid(4, 8, BLUE)]);
        st.segments_mut()[1].stop = 0;
        let out = draw(&st);
        assert!(out[4..].iter().all(|p| *p == BLACK));
    }

    #[test]
    fn reverse_flips_the_segment() {
        let mut seg = rainbow(0, 8);
        seg.reverse = true;
        let mut expected = raw_rainbow(8);
        expected.reverse();
        assert_eq!(draw(&state(&[seg])), expected);
    }

    #[test]
    fn mirror_reflects_the_first_half() {
        let mut seg = rainbow(0, 8);
        seg.mirror = true;
        let out = draw(&state(&[seg]));
        assert_eq!(out[..4], raw_rainbow(4)[..4], "the effect draws half");
        for i in 0..4 {
            assert_eq!(out[i], out[7 - i], "pixel {i}");
        }
    }

    #[test]
    fn an_odd_mirrored_segment_keeps_its_centre() {
        let mut seg = rainbow(0, 5);
        seg.mirror = true;
        let out = draw(&state(&[seg]));
        let drawn = raw_rainbow(3);
        assert_eq!(out[..5], [drawn[0], drawn[1], drawn[2], drawn[1], drawn[0]]);
    }

    #[test]
    fn opacity_fades_the_segment() {
        let mut seg = solid(0, 8, RED);
        seg.opacity = 128;
        let out = draw(&state(&[seg]));
        assert!(out[0].r > 0 && out[0].r < 255);
    }

    #[test]
    fn segments_past_the_canvas_are_clipped() {
        let mut canvas = [BLACK; 4];
        let st = state(&[solid(0, 8, RED)]);
        Compositor::<4>::new().render(&mut canvas, &st, &Ctx::from_millis(0));
        assert!(canvas.iter().all(|p| *p == RED.rgb()));
    }

    #[test]
    fn an_unknown_effect_falls_back_to_solid_colour() {
        let mut seg = solid(0, 8, BLUE);
        seg.effect = EffectId(1);
        assert!(draw(&state(&[seg])).iter().all(|p| *p == BLUE.rgb()));
    }

    #[test]
    fn switching_effects_takes_effect_on_the_next_frame() {
        let mut compositor = Compositor::<4>::new();
        let mut canvas = [BLACK; 8];
        let mut st = state(&[solid(0, 8, RED)]);
        compositor.render(&mut canvas, &st, &Ctx::from_millis(321));
        assert!(canvas.iter().all(|p| *p == RED.rgb()));

        st.segments_mut()[0].effect = RAINBOW;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(321));
        assert_eq!(canvas, raw_rainbow(8));
    }

    #[test]
    fn the_catalogue_is_exactly_what_can_be_drawn() {
        for effect in EffectKind::ALL {
            assert!(CATALOGUE.effects.contains(effect.id()));
        }
        assert!(!CATALOGUE.effects.contains(1), "a gap in the ids");
        assert_eq!(CATALOGUE.effects.end(), u16::from(RAINBOW.0) + 1);
        assert!(CATALOGUE.palettes.contains(0));
        assert_eq!(CATALOGUE.palettes.end(), 1);
    }
}
