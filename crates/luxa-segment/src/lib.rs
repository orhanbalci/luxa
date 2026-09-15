//! The compositor: which effect renders into which pixels.
//!
//! This crate answers one question — given the fixture's [`State`] and a
//! canvas, who draws where. Each active segment runs its own effect over its
//! own range of LEDs, with the segment's settings applied around it: mirroring,
//! reversing and opacity. Brightness is not applied here; that is the output
//! stage's job, once, over the finished frame.
//!
//! Each segment's effect draws into a frame of its own that lives between
//! renders, so effects that draw over what they drew last find it there.
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
//! Compositor::<4, 8>::new().render(&mut canvas, &state, &Ctx::from_millis(0));
//! assert!(canvas.iter().all(|p| *p == Crgb::new(255, 160, 0)));
//! ```

#![no_std]
#![forbid(unsafe_code)]

use luxa_color::{Crgb, nscale8};
use luxa_effect::{Ctx, Effect, EffectKind, PALETTES, Params};
use luxa_msg::{Catalogue, IdSet, Rgbw, Segment, State};

const BLACK: Crgb = Crgb::new(0, 0, 0);

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
    offset: usize,
}

/// What a segment looks like apart from its effect: its colours, and its
/// opacity — `0` when it is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Look {
    colors: [Rgbw; 3],
    opacity: u8,
}

impl Look {
    fn of<const NAME: usize>(segment: &Segment<NAME>) -> Self {
        Self {
            colors: segment.colors,
            opacity: if segment.on { segment.opacity } else { 0 },
        }
    }
}

/// A segment's look, fading linearly from what was shown toward its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fade {
    from: Look,
    to: Look,
    start_ms: u32,
    duration_ms: u16,
}

impl Fade {
    /// Already at `look`.
    const fn settled(look: Look) -> Self {
        Self {
            from: look,
            to: look,
            start_ms: 0,
            duration_ms: 0,
        }
    }

    /// Starts a fade toward `to` from what is shown at `now_ms`, if `to` is
    /// new. The duration is fixed when the fade starts.
    fn retarget(&mut self, to: Look, duration_ms: u16, now_ms: u32) {
        if to != self.to {
            self.from = self.shown(now_ms);
            self.to = to;
            self.start_ms = now_ms;
            self.duration_ms = duration_ms;
        }
    }

    /// The look at `now_ms`.
    fn shown(&self, now_ms: u32) -> Look {
        let elapsed = now_ms.wrapping_sub(self.start_ms);
        let duration = u32::from(self.duration_ms);
        if elapsed >= duration {
            return self.to;
        }
        // Progress through the fade, 0 ..= 65535.
        let progress = (elapsed * u32::from(u16::MAX) / duration) as i32;
        let lerp = |from: u8, to: u8| {
            let from = i32::from(from);
            (from + (i32::from(to) - from) * progress / i32::from(u16::MAX)) as u8
        };
        let color = |i: usize| {
            let (a, b) = (self.from.colors[i], self.to.colors[i]);
            Rgbw::new(
                lerp(a.r, b.r),
                lerp(a.g, b.g),
                lerp(a.b, b.b),
                lerp(a.w, b.w),
            )
        };
        Look {
            colors: [color(0), color(1), color(2)],
            opacity: lerp(self.from.opacity, self.to.opacity),
        }
    }
}

/// One segment's running effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    effect: EffectKind,
    setup: Option<Setup>,
    fade: Option<Fade>,
}

impl Slot {
    const EMPTY: Self = Self {
        effect: EffectKind::Solid(luxa_effect::effects::Solid::new()),
        setup: None,
        fade: None,
    };
}

/// Renders every active segment of a [`State`] into a canvas.
///
/// Each segment slot keeps its own effect instance and its own frame, so
/// effects animate independently and can draw over what they drew last. An
/// effect starts fresh, over black, when its segment switches effect, changes
/// how many pixels the effect draws, or its frame moves in the pool.
///
/// `SEGMENTS` matches the state's segment capacity. `PIXELS` sizes the pool the
/// frames share: active segments claim space in id order, as many pixels as
/// each effect draws, and a segment that does not fit is not drawn. The canvas
/// length is enough when segments do not overlap. A segment that is switched
/// off keeps its space, so the frames after it stay where they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compositor<const SEGMENTS: usize, const PIXELS: usize> {
    slots: [Slot; SEGMENTS],
    frames: [Crgb; PIXELS],
}

impl<const SEGMENTS: usize, const PIXELS: usize> Compositor<SEGMENTS, PIXELS> {
    /// A compositor with no effects running yet.
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; SEGMENTS],
            frames: [BLACK; PIXELS],
        }
    }

    /// Renders one frame of `state` into `canvas`.
    ///
    /// Pixels no active segment covers are black. Where segments overlap, the
    /// later one draws on top; a segment that is off leaves what lies beneath.
    /// Segments reaching past the canvas are clipped to it.
    ///
    /// Colour, opacity and on/off changes fade over the state's
    /// [`change_transition`](State::change_transition) from whatever was shown
    /// when the change arrived; a segment switched off fades out before it
    /// stops drawing. A segment seen for the first time starts as it is.
    pub fn render<const NAME: usize>(
        &mut self,
        canvas: &mut [Crgb],
        state: &State<SEGMENTS, NAME>,
        ctx: &Ctx,
    ) {
        canvas.fill(BLACK);
        let transition_ms = state.change_transition.as_millis();
        let Self { slots, frames } = self;
        let mut claimed = 0;
        for (id, segment) in state.active_segments() {
            let end = usize::from(segment.stop).min(canvas.len());
            let start = usize::from(segment.start).min(end);
            if start == end {
                continue;
            }
            // A mirrored segment's effect draws the first half, which is reflected.
            let len = end - start;
            let drawn = if segment.mirror { len.div_ceil(2) } else { len };
            let offset = claimed;
            claimed += drawn;
            let Some(frame) = frames.get_mut(offset..claimed) else {
                continue;
            };
            render_segment(
                &mut slots[id],
                frame,
                offset,
                &mut canvas[start..end],
                segment,
                ctx,
                transition_ms,
            );
        }
    }
}

impl<const SEGMENTS: usize, const PIXELS: usize> Default for Compositor<SEGMENTS, PIXELS> {
    fn default() -> Self {
        Self::new()
    }
}

fn render_segment<const NAME: usize>(
    slot: &mut Slot,
    frame: &mut [Crgb],
    offset: usize,
    view: &mut [Crgb],
    segment: &Segment<NAME>,
    ctx: &Ctx,
    transition_ms: u16,
) {
    let now_ms = ctx.now_ms();
    let target = Look::of(segment);
    let fade = slot.fade.get_or_insert(Fade::settled(target));
    fade.retarget(target, transition_ms, now_ms);
    let look = fade.shown(now_ms);
    // Off, or faded all the way out: leave what lies beneath.
    if look.opacity == 0 {
        return;
    }
    let len = view.len();
    let drawn = frame.len();

    let setup = Setup {
        effect: segment.effect.0,
        drawn,
        offset,
    };
    if slot.setup != Some(setup) {
        slot.effect = EffectKind::from_id(setup.effect).unwrap_or_default();
        slot.setup = Some(setup);
        frame.fill(BLACK);
    }

    let params = Params {
        speed: segment.speed,
        intensity: segment.intensity,
        colors: look.colors,
    };
    slot.effect.render(frame, ctx, &params);

    let half = &mut view[..drawn];
    half.copy_from_slice(frame);
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
    if look.opacity < u8::MAX {
        nscale8(view, look.opacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_effect::effects::{Rainbow, Stepped};
    use luxa_msg::{EffectId, Layout, TransitionTime};

    type TestState = State<4, 8>;
    type TestCompositor = Compositor<4, 16>;

    const RED: Rgbw = Rgbw::new(255, 0, 0, 0);
    const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);
    const RAINBOW: EffectId = EffectId(EffectKind::Rainbow(Rainbow::new()).id());
    const SCANNER: EffectId = EffectId(40);
    const STEP_MS: u32 = Stepped::interval_ms(128);

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

    fn scanner(start: u16, stop: u16) -> Segment<8> {
        let mut s = solid(start, stop, RED);
        s.effect = SCANNER;
        s.speed = 128;
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
        TestCompositor::new().render(&mut canvas, st, &Ctx::from_millis(321));
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
        TestCompositor::new().render(&mut canvas, &st, &Ctx::from_millis(0));
        assert!(canvas.iter().all(|p| *p == RED.rgb()));
    }

    #[test]
    fn an_unknown_effect_falls_back_to_solid_colour() {
        let mut seg = solid(0, 8, BLUE);
        seg.effect = EffectId(6);
        assert!(draw(&state(&[seg])).iter().all(|p| *p == BLUE.rgb()));
    }

    #[test]
    fn switching_effects_takes_effect_on_the_next_frame() {
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        let mut st = state(&[solid(0, 8, RED)]);
        compositor.render(&mut canvas, &st, &Ctx::from_millis(321));
        assert!(canvas.iter().all(|p| *p == RED.rgb()));

        st.segments_mut()[0].effect = RAINBOW;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(321));
        assert_eq!(canvas, raw_rainbow(8));
    }

    #[test]
    fn an_effect_draws_over_its_last_frame() {
        // The scanner's trail exists only if its frame survives between renders.
        let st = state(&[scanner(0, 8)]);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        for step in 0..4 {
            compositor.render(&mut canvas, &st, &Ctx::from_millis(step * STEP_MS));
        }
        let lit = canvas.iter().filter(|p| !p.is_black()).count();
        assert_eq!(lit, 4, "a head and a fading trail: {canvas:?}");
        assert!(canvas[0].r < canvas[3].r, "the trail fades behind the head");
    }

    #[test]
    fn between_steps_the_frame_holds_still() {
        let st = state(&[scanner(0, 8)]);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        compositor.render(&mut canvas, &st, &Ctx::from_millis(0));
        let first = canvas;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(STEP_MS - 1));
        assert_eq!(canvas, first);
    }

    #[test]
    fn switching_a_segment_off_does_not_restart_the_ones_after_it() {
        let mut st = state(&[solid(0, 4, RED), scanner(4, 8)]);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        compositor.render(&mut canvas, &st, &Ctx::from_millis(0));
        compositor.render(&mut canvas, &st, &Ctx::from_millis(STEP_MS));
        let running = canvas;

        st.segments_mut()[0].on = false;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(STEP_MS));
        assert_eq!(
            canvas[4..],
            running[4..],
            "a restart would show one step, not two"
        );
    }

    fn fading(segments: &[Segment<8>], ms: u16) -> TestState {
        let mut st = state(segments);
        st.change_transition = TransitionTime::from_millis(ms);
        st
    }

    #[test]
    fn colour_changes_fade_over_the_transition() {
        let mut st = fading(&[solid(0, 8, RED)], 1_000);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        compositor.render(&mut canvas, &st, &Ctx::from_millis(0));

        st.segments_mut()[0].colors[0] = BLUE;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(1_000));
        assert_eq!(canvas[0], RED.rgb(), "the fade starts from what was shown");

        compositor.render(&mut canvas, &st, &Ctx::from_millis(1_500));
        assert_eq!(canvas[0], Crgb::new(128, 0, 127), "halfway");

        compositor.render(&mut canvas, &st, &Ctx::from_millis(2_000));
        assert_eq!(canvas[0], BLUE.rgb());
    }

    #[test]
    fn a_segment_switched_off_fades_out_before_it_stops_drawing() {
        let mut st = fading(&[solid(0, 8, RED), solid(2, 4, BLUE)], 1_000);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        compositor.render(&mut canvas, &st, &Ctx::from_millis(0));
        assert_eq!(canvas[2], BLUE.rgb());

        st.segments_mut()[1].on = false;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(100));
        compositor.render(&mut canvas, &st, &Ctx::from_millis(600));
        assert!(
            canvas[2].b > 0 && canvas[2].b < 255,
            "half faded: {:?}",
            canvas[2]
        );

        compositor.render(&mut canvas, &st, &Ctx::from_millis(1_100));
        assert_eq!(canvas[2], RED.rgb(), "gone, showing what lies beneath");
    }

    #[test]
    fn a_zero_transition_changes_at_once() {
        let mut st = fading(&[solid(0, 8, RED)], 0);
        let mut compositor = TestCompositor::new();
        let mut canvas = [BLACK; 8];
        compositor.render(&mut canvas, &st, &Ctx::from_millis(0));
        st.segments_mut()[0].colors[0] = BLUE;
        compositor.render(&mut canvas, &st, &Ctx::from_millis(1));
        assert_eq!(canvas[0], BLUE.rgb());
    }

    #[test]
    fn a_segment_that_does_not_fit_the_pool_is_not_drawn() {
        let st = state(&[solid(0, 4, RED), solid(4, 8, BLUE)]);
        let mut canvas = [BLACK; 8];
        Compositor::<4, 4>::new().render(&mut canvas, &st, &Ctx::from_millis(0));
        assert!(canvas[..4].iter().all(|p| *p == RED.rgb()));
        assert!(canvas[4..].iter().all(|p| *p == BLACK));
    }

    #[test]
    fn the_catalogue_is_exactly_what_can_be_drawn() {
        for effect in EffectKind::ALL {
            assert!(CATALOGUE.effects.contains(effect.id()));
        }
        assert!(!CATALOGUE.effects.contains(6), "a gap in the ids");
        let last = EffectKind::ALL[EffectKind::ALL.len() - 1].id();
        assert_eq!(CATALOGUE.effects.end(), u16::from(last) + 1);
        assert!(CATALOGUE.palettes.contains(0));
        assert_eq!(CATALOGUE.palettes.end(), 1);
    }
}
