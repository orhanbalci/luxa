//! Engine behaviour, one rule per test.
//!
//! Where a rule mirrors a fixture in `tests/fixtures/api`, the test carries the
//! fixture's name after its area prefix.

use super::*;
use luxa_msg::{Direction, IdSet, Origin, Seq, TransitionTime, U8Op};

type TestEngine = Engine<8, 16>;
type Cmd = Command<16>;
type Seg = SegmentPatch<16>;

const LAYOUT: Layout = Layout::new(30);
/// The effect and palette counts of the reference controller the fixtures
/// describe.
const CATALOGUE: Catalogue = Catalogue::contiguous(220, 72);

const ORANGE: Rgbw = Segment::<16>::DEFAULT_COLOR;
const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);
const RED: Rgbw = Rgbw::new(255, 0, 0, 0);

fn engine() -> TestEngine {
    TestEngine::new(LAYOUT, CATALOGUE)
}

fn env(seq: u32, command: Cmd) -> Envelope<16> {
    Envelope::new(Seq(seq), command)
}

fn global(patch: GlobalPatch) -> Cmd {
    Cmd::Global(patch)
}

fn power_op(op: BoolOp) -> Cmd {
    global(GlobalPatch {
        on: Some(op),
        ..GlobalPatch::NONE
    })
}

fn seg(patch: Seg) -> Cmd {
    Cmd::Segment(patch)
}

fn create(id: u8, start: u16, stop: u16) -> Cmd {
    seg(Seg {
        start: Some(start),
        stop: Some(stop),
        ..Seg::for_id(id)
    })
}

fn delete(id: u8) -> Cmd {
    seg(Seg {
        stop: Some(0),
        ..Seg::for_id(id)
    })
}

fn colors(slots: [Option<ColorSpec>; 3]) -> Cmd {
    seg(Seg {
        colors: slots,
        ..Seg::for_id(0)
    })
}

fn apply_all(e: &mut TestEngine, commands: impl IntoIterator<Item = Cmd>) {
    for command in commands {
        e.apply(command);
    }
}

fn segment(e: &TestEngine, id: usize) -> Segment<16> {
    *e.state().segment(id).expect("segment exists")
}

fn active_ids_are(e: &TestEngine, ids: &[usize]) -> bool {
    e.state()
        .active_segments()
        .map(|(id, _)| id)
        .eq(ids.iter().copied())
}

// --- Power and brightness ---------------------------------------------------

#[test]
fn power_off_reports_the_remembered_brightness() {
    let mut e = engine();
    assert!(e.apply(Cmd::power(false)).contains(Changes::BRIGHTNESS));
    assert!(!e.state().is_on());
    assert_eq!(e.state().last_brightness, 128);
}

#[test]
fn power_on_restores_the_remembered_brightness() {
    let mut e = engine();
    apply_all(
        &mut e,
        [Cmd::brightness(90), Cmd::power(false), Cmd::power(true)],
    );
    assert_eq!(e.state().brightness, 90);
}

#[test]
fn power_toggle_switches_off_when_on() {
    let mut e = engine();
    e.apply(power_op(BoolOp::Toggle));
    assert!(!e.state().is_on());
    assert_eq!(e.state().last_brightness, 128);
}

#[test]
fn power_toggle_switches_on_when_off() {
    let mut e = engine();
    apply_all(&mut e, [Cmd::power(false), power_op(BoolOp::Toggle)]);
    assert_eq!(e.state().brightness, 128);
}

#[test]
fn power_toggle_with_brightness_switches_on_at_that_level() {
    let mut e = engine();
    e.apply(Cmd::power(false));
    e.apply(global(GlobalPatch {
        on: Some(BoolOp::Toggle),
        brightness: Some(U8Op::Set(32)),
        ..GlobalPatch::NONE
    }));
    assert!(e.state().is_on());
    assert_eq!(e.state().brightness, 32);
}

#[test]
fn power_brightness_zero_switches_off_and_keeps_the_last_level() {
    let mut e = engine();
    apply_all(&mut e, [Cmd::brightness(90), Cmd::brightness(0)]);
    assert!(!e.state().is_on());
    assert_eq!(e.state().last_brightness, 90);
}

#[test]
fn power_brightness_while_off_switches_on() {
    let mut e = engine();
    apply_all(&mut e, [Cmd::power(false), Cmd::brightness(42)]);
    assert!(e.state().is_on());
    assert_eq!(e.state().brightness, 42);
}

#[test]
fn power_brightness_is_applied_before_power() {
    let mut e = engine();
    e.apply(global(GlobalPatch {
        brightness: Some(U8Op::Set(50)),
        on: Some(BoolOp::Set(false)),
        ..GlobalPatch::NONE
    }));
    assert!(!e.state().is_on());
    assert_eq!(e.state().last_brightness, 50);
}

#[test]
fn power_relative_brightness() {
    let mut e = engine();
    e.apply(global(GlobalPatch {
        brightness: Some(U8Op::Add {
            delta: -10,
            wrap: false,
            bounds: None,
        }),
        ..GlobalPatch::NONE
    }));
    assert_eq!(e.state().brightness, 118);

    // Stepping up from full wraps to zero — which is off.
    e.apply(Cmd::brightness(255));
    e.apply(global(GlobalPatch {
        brightness: Some(U8Op::Cycle {
            direction: Direction::Up,
            bounds: None,
        }),
        ..GlobalPatch::NONE
    }));
    assert!(!e.state().is_on());
    assert_eq!(e.state().last_brightness, 255);
}

#[test]
fn power_transition_sets_the_default_duration() {
    let mut e = engine();
    let patch = GlobalPatch {
        transition: Some(TransitionTime::from_deciseconds(10)),
        ..GlobalPatch::NONE
    };
    assert_eq!(e.apply(global(patch)), Changes::TRANSITION);
    assert_eq!(e.state().transition.as_deciseconds(), 10);
    assert_eq!(
        e.apply(global(patch)),
        Changes::NONE,
        "already that duration"
    );
}

#[test]
fn power_one_shot_transition_is_not_state() {
    let mut e = engine();
    let patch = GlobalPatch {
        transition_once: Some(TransitionTime::ZERO),
        ..GlobalPatch::NONE
    };
    assert_eq!(e.apply(global(patch)), Changes::NONE);
    assert_eq!(e.state().transition, TransitionTime::DEFAULT);
}

#[test]
fn power_one_shot_transition_shapes_its_own_batch_only() {
    let mut e = engine();
    let quick = GlobalPatch {
        brightness: Some(U8Op::Set(10)),
        transition_once: Some(TransitionTime::ZERO),
        ..GlobalPatch::NONE
    };
    e.apply_batch([global(quick), Cmd::brightness(20)]);
    assert_eq!(
        e.state().change_transition,
        TransitionTime::ZERO,
        "every command in the batch"
    );

    e.apply_batch([Cmd::brightness(30)]);
    assert_eq!(e.state().change_transition, TransitionTime::DEFAULT);

    e.apply(global(quick));
    assert_eq!(e.state().change_transition, TransitionTime::ZERO);
    e.apply(Cmd::brightness(40));
    assert_eq!(
        e.state().change_transition,
        TransitionTime::DEFAULT,
        "a single command is a change of its own"
    );
}

#[test]
fn power_a_new_default_transition_shapes_its_own_change_and_later_ones() {
    let mut e = engine();
    let slow = TransitionTime::from_deciseconds(20);
    e.apply(global(GlobalPatch {
        transition: Some(slow),
        ..GlobalPatch::NONE
    }));
    assert_eq!(e.state().change_transition, slow);
    e.apply(Cmd::brightness(99));
    assert_eq!(e.state().change_transition, slow);
}

#[test]
fn power_redundant_commands_report_no_change() {
    let mut e = engine();
    assert!(e.apply(Cmd::power(true)).is_empty(), "already on");
    assert!(!e.apply(Cmd::brightness(7)).is_empty());
    assert!(e.apply(Cmd::brightness(7)).is_empty());
    assert!(!e.apply(Cmd::power(false)).is_empty());
    assert!(e.apply(Cmd::power(false)).is_empty(), "already off");
    assert!(e.apply(Cmd::brightness(0)).is_empty(), "already off");
    assert!(e.apply(global(GlobalPatch::NONE)).is_empty());
}

// --- Segment lifecycle ------------------------------------------------------

#[test]
fn segment_create_appends_a_selected_orange_segment() {
    let mut e = engine();
    assert!(e.apply(create(1, 10, 20)).contains(Changes::SEGMENT_BOUNDS));

    let s = segment(&e, 1);
    assert_eq!((s.start, s.stop), (10, 20));
    assert!(s.on && s.selected && !s.reverse && !s.mirror);
    assert_eq!(s.opacity, 255);
    assert_eq!(s.colors, [ORANGE, Rgbw::BLACK, Rgbw::BLACK]);
    assert_eq!((s.effect, s.speed, s.intensity), (EffectId(0), 128, 128));
    assert_eq!(s.palette, PaletteId(0));
    assert_eq!(s.caps, LightCaps::RGB);
    assert!(s.name.is_empty());

    assert_eq!((segment(&e, 0).start, segment(&e, 0).stop), (0, 30));
}

#[test]
fn segment_create_appends_at_the_end_whatever_id_is_asked() {
    let mut e = engine();
    e.apply(create(5, 10, 20));
    assert_eq!(e.state().segments().len(), 2);
    assert_eq!(segment(&e, 1).start, 10);
}

#[test]
fn segment_an_unknown_id_without_stop_is_ignored() {
    let mut e = engine();
    let changes = e.apply(seg(Seg {
        effect: Some(U8Op::Set(3)),
        ..Seg::for_id(5)
    }));
    assert_eq!(changes, Changes::NONE);
    assert_eq!(e.state().segments().len(), 1);
    assert_eq!(segment(&e, 0).effect, EffectId(0));
}

#[test]
fn segment_ids_beyond_capacity_are_ignored() {
    let mut e = engine();
    assert_eq!(e.apply(create(8, 0, 5)), Changes::NONE);
    assert_eq!(e.state().segments().len(), 1);
}

#[test]
fn segment_object_without_id_applies_to_every_selected_segment() {
    let mut e = engine();
    e.apply(create(1, 10, 20));
    e.apply(seg(Seg {
        effect: Some(U8Op::Set(8)),
        speed: Some(U8Op::Set(200)),
        intensity: Some(U8Op::Set(50)),
        palette: Some(U8Op::Set(11)),
        ..Seg::for_selected()
    }));
    for id in [0, 1] {
        let s = segment(&e, id);
        assert_eq!(
            (s.effect, s.speed, s.intensity, s.palette),
            (EffectId(8), 200, 50, PaletteId(11)),
            "segment {id}"
        );
    }
}

#[test]
fn segment_unselected_segments_are_skipped_by_fan_out() {
    let mut e = engine();
    e.apply(create(1, 10, 20));
    e.apply(seg(Seg {
        selected: Some(BoolOp::Set(false)),
        ..Seg::for_id(1)
    }));
    e.apply(seg(Seg {
        speed: Some(U8Op::Set(10)),
        ..Seg::for_selected()
    }));
    assert_eq!(segment(&e, 0).speed, 10);
    assert_eq!(segment(&e, 1).speed, 128);
}

#[test]
fn segment_object_with_id_touches_only_that_segment() {
    let mut e = engine();
    e.apply(create(1, 10, 20));
    e.apply(seg(Seg {
        effect: Some(U8Op::Set(2)),
        ..Seg::for_id(1)
    }));
    assert_eq!(segment(&e, 0).effect, EffectId(0));
    assert_eq!(segment(&e, 1).effect, EffectId(2));
}

#[test]
fn segment_len_sets_stop_when_stop_is_absent() {
    let mut e = engine();
    e.apply(seg(Seg {
        start: Some(5),
        len: Some(10),
        ..Seg::for_id(0)
    }));
    assert_eq!((segment(&e, 0).start, segment(&e, 0).stop), (5, 15));
}

#[test]
fn segment_stop_is_clamped_to_the_strip() {
    let mut e = engine();
    e.apply(seg(Seg {
        start: Some(10),
        stop: Some(100),
        ..Seg::for_id(0)
    }));
    assert_eq!((segment(&e, 0).start, segment(&e, 0).stop), (10, 30));
}

#[test]
fn segment_a_start_past_the_strip_keeps_the_old_start() {
    let mut e = engine();
    e.apply(seg(Seg {
        start: Some(35),
        stop: Some(50),
        ..Seg::for_id(0)
    }));
    assert_eq!((segment(&e, 0).start, segment(&e, 0).stop), (0, 30));
}

#[test]
fn segment_stop_zero_deletes_the_segment() {
    let mut e = engine();
    e.apply(create(1, 10, 20));
    assert!(e.apply(delete(1)).contains(Changes::SEGMENT_BOUNDS));
    assert_eq!(e.state().segments().len(), 2, "the slot stays");
    assert!(!segment(&e, 1).is_active());
    assert!(active_ids_are(&e, &[0]));
}

#[test]
fn segment_a_deleted_segment_keeps_its_id_slot() {
    let mut e = engine();
    apply_all(&mut e, [create(1, 10, 20), delete(1), create(2, 20, 30)]);
    assert!(active_ids_are(&e, &[0, 2]));
    assert_eq!(segment(&e, 2).colors[0], ORANGE);
}

#[test]
fn segment_reactivating_a_deleted_slot_keeps_its_colours() {
    let mut e = engine();
    apply_all(
        &mut e,
        [
            create(1, 10, 20),
            seg(Seg {
                colors: [Some(ColorSpec::Rgbw(BLUE)), None, None],
                ..Seg::for_id(1)
            }),
            delete(1),
            seg(Seg {
                start: Some(0),
                stop: Some(5),
                ..Seg::for_id(1)
            }),
        ],
    );
    let s = segment(&e, 1);
    assert_eq!((s.start, s.stop), (0, 5));
    assert_eq!(s.colors[0], BLUE);
}

#[test]
fn segment_a_deleted_segment_ignores_other_changes() {
    let mut e = engine();
    apply_all(&mut e, [create(1, 10, 20), delete(1)]);
    let changes = e.apply(seg(Seg {
        effect: Some(U8Op::Set(3)),
        ..Seg::for_id(1)
    }));
    assert_eq!(changes, Changes::NONE);
    assert_eq!(segment(&e, 1).effect, EffectId(0));
}

#[test]
fn segment_deleting_the_main_segment_resets_it() {
    let mut state = State::new(LAYOUT);
    state.push_segment(Segment::new(10, 20)).unwrap();
    state.main_segment = 1;
    let mut e = TestEngine::with_state(LAYOUT, CATALOGUE, state);
    e.apply(delete(1));
    assert_eq!(e.state().main_segment, 0);
}

#[test]
fn segment_opacity() {
    let mut e = engine();
    let changes = e.apply(seg(Seg {
        opacity: Some(U8Op::Set(100)),
        ..Seg::for_id(0)
    }));
    assert_eq!(changes, Changes::SEGMENT_OPACITY);
    assert!(segment(&e, 0).on);
    assert_eq!(segment(&e, 0).opacity, 100);
}

#[test]
fn segment_opacity_zero_switches_off_and_keeps_opacity() {
    let mut e = engine();
    e.apply(seg(Seg {
        opacity: Some(U8Op::Set(0)),
        ..Seg::for_id(0)
    }));
    assert!(!segment(&e, 0).on);
    assert_eq!(segment(&e, 0).opacity, 255);
}

#[test]
fn segment_power() {
    let mut e = engine();
    e.apply(seg(Seg {
        on: Some(BoolOp::Set(false)),
        ..Seg::for_id(0)
    }));
    assert!(!segment(&e, 0).on);
    assert!(e.state().is_on(), "the fixture stays on");
    e.apply(seg(Seg {
        on: Some(BoolOp::Toggle),
        ..Seg::for_id(0)
    }));
    assert!(segment(&e, 0).on);
}

#[test]
fn segment_reverse_and_mirror() {
    let mut e = engine();
    let changes = e.apply(seg(Seg {
        reverse: Some(BoolOp::Set(true)),
        mirror: Some(BoolOp::Toggle),
        ..Seg::for_id(0)
    }));
    assert_eq!(changes, Changes::SEGMENT_OPTIONS);
    assert!(segment(&e, 0).reverse && segment(&e, 0).mirror);
}

#[test]
fn segment_name_is_set_and_cleared_by_an_empty_name() {
    let mut e = engine();
    let changes = e.apply(seg(Seg {
        name: Some(Name::new("Desk")),
        ..Seg::for_id(0)
    }));
    assert_eq!(changes, Changes::SEGMENT_NAME);
    assert_eq!(segment(&e, 0).name.as_str(), "Desk");

    e.apply(seg(Seg {
        name: Some(Name::EMPTY),
        ..Seg::for_id(0)
    }));
    assert!(segment(&e, 0).name.is_empty());
}

#[test]
fn segment_name_is_cleared_when_bounds_change_without_a_name() {
    let mut e = engine();
    e.apply(seg(Seg {
        name: Some(Name::new("Desk")),
        ..Seg::for_id(0)
    }));
    e.apply(seg(Seg {
        stop: Some(20),
        ..Seg::for_id(0)
    }));
    assert_eq!(segment(&e, 0).stop, 20);
    assert!(segment(&e, 0).name.is_empty());
}

#[test]
fn segment_selection_and_naming_publish_without_origins() {
    let mut e = engine();
    e.apply(create(1, 10, 20));
    let outcome = e
        .apply_batch([
            seg(Seg {
                selected: Some(BoolOp::Set(false)),
                ..Seg::for_id(1)
            }),
            seg(Seg {
                name: Some(Name::new("Shelf")),
                ..Seg::for_id(0)
            }),
        ])
        .expect("published so reads stay current");
    assert_eq!(
        outcome.changes,
        Changes::SEGMENT_SELECTION | Changes::SEGMENT_NAME
    );
    assert!(!outcome.changes.is_state_change());
    assert!(outcome.origins.is_empty(), "nothing to announce");
}

#[test]
fn segment_effect_gaps_skip_forward_and_overflow_falls_back() {
    let catalogue = Catalogue::new(IdSet::EMPTY.with(2).with(5), IdSet::EMPTY);
    let mut e = TestEngine::new(LAYOUT, catalogue);
    let set_fx = |e: &mut TestEngine, fx| {
        e.apply(seg(Seg {
            effect: Some(U8Op::Set(fx)),
            ..Seg::for_id(0)
        }));
        segment(e, 0).effect
    };
    assert_eq!(set_fx(&mut e, 3), EffectId(5), "gap skips forward");
    assert_eq!(set_fx(&mut e, 1), EffectId(2));
    assert_eq!(set_fx(&mut e, 6), EffectId(0), "past the last falls back");
}

#[test]
fn segment_stepping_past_the_last_effect_wraps_to_zero() {
    let mut e = engine();
    e.apply(seg(Seg {
        effect: Some(U8Op::Set(219)),
        ..Seg::for_id(0)
    }));
    e.apply(seg(Seg {
        effect: Some(U8Op::Cycle {
            direction: Direction::Up,
            bounds: None,
        }),
        ..Seg::for_id(0)
    }));
    assert_eq!(segment(&e, 0).effect, EffectId(0));
}

#[test]
fn segment_random_effects_are_valid() {
    for seed in 0..50 {
        let mut e = engine().with_seed(seed);
        e.apply(seg(Seg {
            effect: Some(U8Op::Random { bounds: None }),
            ..Seg::for_id(0)
        }));
        assert!(segment(&e, 0).effect.0 < 220);
    }
}

#[test]
fn segment_invalid_palettes_fall_back_to_zero() {
    let mut e = engine();
    let set_pal = |e: &mut TestEngine, pal| {
        e.apply(seg(Seg {
            palette: Some(U8Op::Set(pal)),
            ..Seg::for_id(0)
        }));
        segment(e, 0).palette
    };
    assert_eq!(set_pal(&mut e, 11), PaletteId(11));
    assert_eq!(set_pal(&mut e, 100), PaletteId(0));
}

#[test]
fn segment_palette_is_ignored_without_colour() {
    let mut e = TestEngine::new(Layout::with_caps(30, LightCaps::WHITE), CATALOGUE);
    e.apply(seg(Seg {
        palette: Some(U8Op::Set(11)),
        ..Seg::for_id(0)
    }));
    assert_eq!(segment(&e, 0).palette, PaletteId(0));
}

#[test]
fn segment_custom_sliders_and_checkboxes() {
    let mut e = engine();
    let changes = e.apply(seg(Seg {
        custom: [Some(U8Op::Set(7)), None, Some(U8Op::Set(200))],
        checks: [Some(BoolOp::Set(true)), None, Some(BoolOp::Toggle)],
        ..Seg::for_id(0)
    }));
    assert!(changes.contains(Changes::SEGMENT_EFFECT));
    let s = segment(&e, 0);
    assert_eq!(s.custom, [7, 128, 31], "custom slider 3 stops at 31");
    assert_eq!(s.checks, [true, false, true]);

    e.apply(seg(Seg {
        custom: [
            Some(U8Op::Cycle {
                direction: Direction::Down,
                bounds: None,
            }),
            None,
            Some(U8Op::Cycle {
                direction: Direction::Up,
                bounds: None,
            }),
        ],
        ..Seg::for_id(0)
    }));
    let s = segment(&e, 0);
    assert_eq!(s.custom[0], 6);
    assert_eq!(s.custom[2], 0, "custom slider 3 cycles within 0..=31");
}

#[test]
fn segment_random_custom_slider_3_stays_in_range() {
    for seed in 0..50 {
        let mut e = engine().with_seed(seed);
        e.apply(seg(Seg {
            custom: [None, None, Some(U8Op::Random { bounds: None })],
            ..Seg::for_id(0)
        }));
        assert!(segment(&e, 0).custom[2] <= 31);
    }
}

// --- Colours ----------------------------------------------------------------

#[test]
fn color_channel_arrays_set_slots_in_order() {
    let mut e = engine();
    let changes = e.apply(colors([
        Some(ColorSpec::Rgbw(BLUE)),
        Some(ColorSpec::Rgbw(RED)),
        None,
    ]));
    assert_eq!(changes, Changes::SEGMENT_COLORS);
    assert_eq!(segment(&e, 0).colors, [BLUE, RED, Rgbw::BLACK]);
}

#[test]
fn color_hex_channel_object_and_kelvin() {
    let mut e = engine();
    e.apply(colors([
        Some(ColorSpec::Rgbw(Rgbw::new(0, 255, 0, 0))),
        Some(ColorSpec::Partial {
            r: None,
            g: Some(7),
            b: None,
            w: None,
        }),
        Some(ColorSpec::Kelvin(2700)),
    ]));
    assert_eq!(
        segment(&e, 0).colors,
        [
            Rgbw::new(0, 255, 0, 0),
            Rgbw::new(0, 7, 0, 0),
            Rgbw::new(255, 167, 87, 0)
        ]
    );
}

#[test]
fn color_zero_is_black_and_an_absent_slot_is_kept() {
    let mut e = engine();
    e.apply(colors([
        Some(ColorSpec::Rgbw(Rgbw::new(1, 2, 3, 0))),
        Some(ColorSpec::Rgbw(Rgbw::new(4, 5, 6, 0))),
        Some(ColorSpec::Rgbw(Rgbw::new(7, 8, 9, 0))),
    ]));
    e.apply(colors([
        Some(ColorSpec::Rgbw(Rgbw::BLACK)),
        None,
        Some(ColorSpec::Kelvin(10000)),
    ]));
    assert_eq!(
        segment(&e, 0).colors,
        [
            Rgbw::BLACK,
            Rgbw::new(4, 5, 6, 0),
            Rgbw::new(202, 218, 255, 0)
        ]
    );
}

#[test]
fn color_channel_object_keeps_unmentioned_channels() {
    let mut e = engine();
    e.apply(colors([
        Some(ColorSpec::Partial {
            r: None,
            g: None,
            b: Some(50),
            w: None,
        }),
        None,
        None,
    ]));
    assert_eq!(segment(&e, 0).colors[0], Rgbw::new(255, 160, 50, 0));
}

#[test]
fn color_kelvin_zero_is_black() {
    let mut e = engine();
    e.apply(colors([Some(ColorSpec::Kelvin(0)), None, None]));
    assert_eq!(segment(&e, 0).colors[0], Rgbw::BLACK);
}

#[test]
fn color_random_colours_are_reproducible_from_the_seed() {
    let pick = |seed| {
        let mut e = engine().with_seed(seed);
        e.apply(colors([Some(ColorSpec::Random), None, None]));
        segment(&e, 0).colors[0]
    };
    assert_eq!(pick(9), pick(9));
    assert_eq!(pick(9).w, 0);
    assert!(!pick(9).is_black());
}

#[test]
fn color_segments_without_colour_or_white_only_switch() {
    let mut e = TestEngine::new(Layout::with_caps(30, LightCaps::NONE), CATALOGUE);
    e.apply(colors([Some(ColorSpec::Rgbw(RED)), None, None]));
    assert_eq!(
        segment(&e, 0).colors[..2],
        [Rgbw::new(255, 255, 255, 255), Rgbw::BLACK]
    );
}

#[test]
fn color_white_only_segments_take_colours() {
    let mut e = TestEngine::new(Layout::with_caps(30, LightCaps::WHITE), CATALOGUE);
    e.apply(colors([
        Some(ColorSpec::Rgbw(Rgbw::new(0, 0, 0, 200))),
        None,
        None,
    ]));
    assert_eq!(segment(&e, 0).colors[0], Rgbw::new(0, 0, 0, 200));
}

// --- Compaction -------------------------------------------------------------

fn four_segments() -> TestEngine {
    let mut e = engine();
    apply_all(
        &mut e,
        [create(1, 10, 20), create(2, 20, 25), create(3, 25, 30)],
    );
    e
}

#[test]
fn compact_a_small_deletion_keeps_ids() {
    let mut e = four_segments();
    e.apply(delete(1));
    assert_eq!(e.apply(Cmd::CompactSegments { deleted: 1 }), Changes::NONE);
    assert_eq!(e.state().segments().len(), 4);
}

#[test]
fn compact_deleting_half_of_more_than_three_closes_the_gaps() {
    let mut e = four_segments();
    apply_all(&mut e, [delete(1), delete(2)]);
    assert_eq!(
        e.apply(Cmd::CompactSegments { deleted: 2 }),
        Changes::SEGMENT_BOUNDS
    );
    assert_eq!(e.state().segments().len(), 2);
    assert_eq!(segment(&e, 1).start, 25, "old id 3 is now id 1");
    assert_eq!(e.state().main_segment, 0);
}

#[test]
fn compact_three_or_fewer_segments_never_compact() {
    let mut e = engine();
    apply_all(
        &mut e,
        [create(1, 10, 20), create(2, 20, 30), delete(1), delete(2)],
    );
    assert_eq!(e.apply(Cmd::CompactSegments { deleted: 2 }), Changes::NONE);
    assert_eq!(e.state().segments().len(), 3);
}

// --- Batches, acknowledgement and origins -----------------------------------

#[test]
fn batch_starts_in_a_freshly_booted_state() {
    let e = engine();
    assert_eq!(e.state(), &State::new(LAYOUT));
    assert_eq!(e.layout(), LAYOUT);
    assert_eq!(e.catalogue(), CATALOGUE);
}

#[test]
fn batch_publishes_once_with_the_final_value() {
    let mut e = engine();
    let published = e
        .apply_batch([
            Cmd::brightness(10),
            Cmd::brightness(20),
            Cmd::power(false),
            Cmd::brightness(30),
        ])
        .expect("state changed, so a publish is due")
        .state
        .clone();
    assert_eq!(published.brightness, 30);
    assert_eq!(&published, e.state());
}

#[test]
fn batch_empty_publishes_nothing() {
    assert_eq!(engine().apply_batch::<Cmd>([]), None);
}

#[test]
fn batch_of_no_ops_publishes_nothing() {
    let mut e = engine();
    let level = e.state().brightness;
    assert_eq!(
        e.apply_batch([Cmd::power(true), Cmd::brightness(level)]),
        None
    );
}

#[test]
fn batch_bare_commands_leave_applied_seq_alone() {
    let mut e = engine();
    e.apply_batch([env(5, Cmd::brightness(1))]);
    e.apply_batch([Cmd::brightness(2)]);
    assert_eq!(e.state().brightness, 2, "applied like any other");
    assert_eq!(e.state().applied_seq, Seq(5), "but not numbered");
}

#[test]
fn batch_that_nets_out_to_no_change_still_publishes() {
    // Intermediate states are not observable, but the engine must not try to
    // be clever about it: any step that moved counts, and a redundant publish
    // is far safer than a dropped one.
    let mut e = engine();
    let before = e.state().brightness;
    let published = e.apply_batch([Cmd::brightness(1), Cmd::brightness(before)]);
    assert_eq!(published.map(|o| o.state.brightness), Some(before));
}

#[test]
fn batch_applies_every_command() {
    let mut e = engine();
    e.apply_batch([Cmd::brightness(3), Cmd::power(false), create(1, 10, 20)]);
    assert_eq!(e.state().last_brightness, 3);
    assert!(!e.state().is_on());
    assert!(active_ids_are(&e, &[0, 1]));
}

#[test]
fn batch_a_no_op_awaiting_a_reply_still_publishes() {
    let mut e = engine();
    let published = e
        .apply_batch([Envelope::awaiting_reply(Seq(1), Cmd::power(true))])
        .expect("someone is waiting, so a publish is due even without a change");
    assert!(published.state.has_applied(Seq(1)));
    assert!(published.changes.is_empty());
    assert!(published.origins.is_empty(), "nothing to announce");
}

#[test]
fn batch_applied_seq_advances_even_when_nothing_is_published() {
    let mut e = engine();
    assert_eq!(e.apply_batch([env(7, Cmd::power(true))]), None);
    assert_eq!(e.state().applied_seq, Seq(7));
}

#[test]
fn batch_a_waiter_is_not_satisfied_by_an_earlier_batch() {
    let mut e = engine();
    // The waiter's command is Seq(3), still queued behind this batch.
    let early = e
        .apply_batch([env(1, Cmd::brightness(10)), env(2, Cmd::brightness(20))])
        .expect("state changed");
    assert!(!early.state.has_applied(Seq(3)));

    let late = e
        .apply_batch([Envelope::awaiting_reply(Seq(3), Cmd::brightness(20))])
        .expect("awaited");
    assert!(late.state.has_applied(Seq(3)));
}

#[test]
fn batch_origins_record_only_commands_that_changed_state() {
    let mut e = engine();
    let outcome = e
        .apply_batch([
            env(1, Cmd::brightness(10)).with_origin(Origin::Notification),
            env(2, Cmd::power(true)).with_origin(Origin::Button), // already on
        ])
        .expect("state changed");
    assert!(outcome.origins.contains(Origin::Notification));
    assert!(!outcome.origins.contains(Origin::Button));
    assert!(
        !outcome.origins.notifies_peers(),
        "a change received from a peer must not be echoed back"
    );
}

#[test]
fn batch_a_direct_change_is_announced() {
    let mut e = engine();
    let outcome = e.apply_batch([Cmd::brightness(10)]).expect("changed");
    assert!(outcome.origins.contains(Origin::Direct));
    assert!(outcome.origins.notifies_peers());
}

#[test]
fn batch_state_can_be_restored() {
    let mut state = State::new(LAYOUT);
    state.brightness = 0;
    state.last_brightness = 9;
    let e = TestEngine::with_state(LAYOUT, CATALOGUE, state.clone());
    assert_eq!(e.state(), &state);
}
