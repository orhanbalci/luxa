use super::*;
use luxa_msg::{Bounds, Direction};

type Request = StateRequest<4, 16>;

fn parse(json: &str) -> Request {
    let mut body = [0u8; 512];
    body[..json.len()].copy_from_slice(json.as_bytes());
    parse_state(&mut body[..json.len()]).unwrap_or_else(|e| panic!("{json}: {e:?}"))
}

fn fails(json: &str) -> bool {
    let mut body = [0u8; 512];
    body[..json.len()].copy_from_slice(json.as_bytes());
    parse_state::<4, 16>(&mut body[..json.len()]).is_err()
}

fn only_segment(json: &str) -> SegmentPatch<16> {
    let request = parse(json);
    let [segment] = request.segments() else {
        panic!("{json}: expected one segment, got {:?}", request.segments())
    };
    *segment
}

fn colors(json: &str) -> [Option<ColorSpec>; 3] {
    only_segment(&format_seg_col(json)).colors
}

fn format_seg_col(col: &str) -> String_ {
    let mut s = String_::new();
    s.push_str(r#"{"seg":{"col":"#);
    s.push_str(col);
    s.push_str("}}");
    s
}

/// A tiny fixed-capacity string so the tests stay `no_std`-friendly.
struct String_ {
    buf: [u8; 256],
    len: usize,
}

impl String_ {
    const fn new() -> Self {
        Self {
            buf: [0; 256],
            len: 0,
        }
    }
    fn push_str(&mut self, s: &str) {
        self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
    }
}

impl core::ops::Deref for String_ {
    type Target = str;
    fn deref(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap()
    }
}

const fn rgbw(r: u8, g: u8, b: u8, w: u8) -> Option<ColorSpec> {
    Some(ColorSpec::Rgbw(Rgbw::new(r, g, b, w)))
}

// --- Envelope of the request --------------------------------------------------

#[test]
fn only_objects_are_requests() {
    assert!(fails(""));
    assert!(fails("[]"));
    assert!(fails("true"));
    assert!(fails(r#"{"on":tru}"#));
    assert!(!fails("{}"));
}

#[test]
fn trailing_content_after_the_object_is_ignored() {
    assert_eq!(
        parse(r#"{"bri":5} tail"#).global.brightness,
        Some(U8Op::Set(5))
    );
}

#[test]
fn deep_nesting_is_rejected() {
    let ten = r#"{"a":[[[[[[[[[1]]]]]]]]]}"#;
    let eleven = r#"{"a":[[[[[[[[[[1]]]]]]]]]]}"#;
    assert!(!fails(ten));
    assert!(fails(eleven));
    assert!(
        !fails(r#"{"n":"[[[[[[[[[[[["}"#),
        "brackets in strings do not count"
    );
}

#[test]
fn unknown_keys_are_ignored() {
    let r = parse(r#"{"lor":2,"nl":{"on":true,"dur":[1,{"x":null}]},"on":true,"time":1700000000}"#);
    assert_eq!(r.global.on, Some(BoolOp::Set(true)));
}

#[test]
fn the_first_duplicate_key_wins() {
    let r = parse(r#"{"bri":10,"bri":20,"on":"x","on":true}"#);
    assert_eq!(r.global.brightness, Some(U8Op::Set(10)));
    assert_eq!(r.global.on, None, "the first value was not a switch");
}

#[test]
fn verbose_is_only_true() {
    assert!(parse(r#"{"v":true}"#).verbose);
    assert!(!parse(r#"{"v":1}"#).verbose);
    assert!(!parse("{}").verbose);
}

// --- Fixture-wide keys --------------------------------------------------------

#[test]
fn power_switches() {
    assert_eq!(parse(r#"{"on":true}"#).global.on, Some(BoolOp::Set(true)));
    assert_eq!(parse(r#"{"on":"t"}"#).global.on, Some(BoolOp::Toggle));
    assert_eq!(parse(r#"{"on":"toggle"}"#).global.on, Some(BoolOp::Toggle));
    assert_eq!(parse(r#"{"on":"no"}"#).global.on, None);
    assert_eq!(parse(r#"{"on":1}"#).global.on, None);
}

#[test]
fn levels() {
    let level = |json| parse(json).global.brightness;
    assert_eq!(level(r#"{"bri":128}"#), Some(U8Op::Set(128)));
    assert_eq!(
        level(r#"{"bri":300}"#),
        Some(U8Op::Set(0)),
        "out of range reads as 0"
    );
    assert_eq!(level(r#"{"bri":-1}"#), None);
    assert_eq!(level(r#"{"bri":1.5}"#), None);
    assert_eq!(level(r#"{"bri":null}"#), None);
    assert_eq!(
        level(r#"{"bri":"~-10"}"#),
        Some(U8Op::Add {
            delta: -10,
            wrap: false,
            bounds: None
        })
    );
    assert_eq!(
        level(r#"{"bri":"1~5~"}"#),
        Some(U8Op::Cycle {
            direction: Direction::Up,
            bounds: Some(Bounds::new(1, 5))
        })
    );
}

#[test]
fn transitions() {
    let r = parse(r#"{"transition":10,"tt":0}"#);
    assert_eq!(
        r.global.transition,
        Some(TransitionTime::from_deciseconds(10))
    );
    assert_eq!(r.global.transition_once, Some(TransitionTime::ZERO));
    assert_eq!(parse(r#"{"transition":-1}"#).global.transition, None);
    assert_eq!(parse(r#"{"transition":"10"}"#).global.transition, None);
}

// --- Segments -----------------------------------------------------------------

#[test]
fn an_object_without_id_targets_the_selected_segments() {
    assert_eq!(
        only_segment(r#"{"seg":{"fx":8}}"#).target,
        SegmentTarget::Selected
    );
    assert_eq!(
        only_segment(r#"{"seg":{"id":-1,"fx":8}}"#).target,
        SegmentTarget::Selected
    );
    assert_eq!(
        only_segment(r#"{"seg":{"id":2,"fx":8}}"#).target,
        SegmentTarget::Id(2)
    );
}

#[test]
fn array_entries_take_their_id_or_their_position() {
    let r = parse(r#"{"seg":[{"fx":1},7,{"id":3},{"id":300}]}"#);
    let targets: [SegmentTarget; 3] = [
        r.segments()[0].target,
        r.segments()[1].target,
        r.segments()[2].target,
    ];
    assert_eq!(
        targets,
        [
            SegmentTarget::Id(0),
            SegmentTarget::Id(3),
            SegmentTarget::Id(3)
        ],
        "the non-object at position 1 is skipped; id 300 does not fit, so position 3 is used"
    );
}

#[test]
fn array_deletions_are_counted_for_compaction() {
    let r = parse(r#"{"seg":[{"id":1,"stop":0},{"id":2,"stop":0},{"id":3,"stop":5}]}"#);
    let mut commands = r.commands();
    assert!(matches!(commands.next(), Some(Command::Segment(_))));
    assert!(matches!(commands.nth(1), Some(Command::Segment(_))));
    assert_eq!(
        commands.next(),
        Some(Command::CompactSegments { deleted: 2 })
    );
    assert_eq!(commands.next(), None);

    let object = parse(r#"{"seg":{"id":1,"stop":0}}"#);
    assert!(
        !object
            .commands()
            .any(|c| matches!(c, Command::CompactSegments { .. })),
        "only arrays compact"
    );
}

#[test]
fn commands_come_in_application_order() {
    let r = parse(r#"{"seg":[{"fx":9}],"bri":10}"#);
    let mut commands = r.commands();
    assert!(
        matches!(commands.next(), Some(Command::Global(_))),
        "fixture-wide first"
    );
    assert!(matches!(commands.next(), Some(Command::Segment(_))));
    assert_eq!(
        commands.next(),
        Some(Command::CompactSegments { deleted: 0 })
    );
    assert_eq!(commands.next(), None);

    assert_eq!(parse(r#"{"v":true}"#).commands().count(), 0);
}

#[test]
fn entries_past_the_capacity_are_dropped() {
    let r = parse(r#"{"seg":[{},{},{},{},{"fx":1}]}"#);
    assert_eq!(r.segments().len(), 4);
}

#[test]
fn segment_bounds() {
    let s = only_segment(r#"{"seg":{"id":0,"start":5,"stop":20,"len":10}}"#);
    assert_eq!((s.start, s.stop, s.len), (Some(5), Some(20), Some(10)));

    let s = only_segment(r#"{"seg":{"id":0,"start":70000,"stop":-1,"len":0}}"#);
    assert_eq!((s.start, s.stop, s.len), (None, None, None));

    let s = only_segment(r#"{"seg":{"id":0,"stop":70000,"len":2.9}}"#);
    assert_eq!(
        s.stop,
        Some(70000u32 as u16),
        "truncated like the reference"
    );
    assert_eq!(s.len, Some(2), "a float length is truncated");
}

#[test]
fn segment_names() {
    let name = |json| only_segment(json).name;
    assert_eq!(name(r#"{"seg":{"n":"Desk"}}"#), Some(Name::new("Desk")));
    assert_eq!(name(r#"{"seg":{"n":""}}"#), Some(Name::EMPTY));
    assert_eq!(
        name(r#"{"seg":{"n":1}}"#),
        Some(Name::EMPTY),
        "a true non-string clears"
    );
    assert_eq!(name(r#"{"seg":{"n":false}}"#), None);
    assert_eq!(name(r#"{"seg":{"n":null}}"#), None);
    assert_eq!(
        name(r#"{"seg":{"n":"Desk \"left\" and a long tail"}}"#),
        Some(Name::new(r#"Desk "left" and "#)),
        "unescaped, then truncated to the capacity"
    );
}

#[test]
fn segment_switches_and_levels() {
    let s = only_segment(
        r#"{"seg":{"on":"t","bri":100,"fx":"~","sx":"r","ix":300,"pal":11,"sel":false,"rev":true,"mi":"t","c1":5,"c3":"~","o1":true,"o3":"t","bm":7}}"#,
    );
    assert_eq!(s.on, Some(BoolOp::Toggle));
    assert_eq!(s.opacity, Some(U8Op::Set(100)));
    assert_eq!(
        s.effect,
        Some(U8Op::Cycle {
            direction: Direction::Up,
            bounds: None
        })
    );
    assert_eq!(s.speed, Some(U8Op::Random { bounds: None }));
    assert_eq!(s.intensity, Some(U8Op::Set(0)));
    assert_eq!(s.palette, Some(U8Op::Set(11)));
    assert_eq!(s.selected, Some(BoolOp::Set(false)));
    assert_eq!(s.reverse, Some(BoolOp::Set(true)));
    assert_eq!(s.mirror, Some(BoolOp::Toggle));
    assert_eq!(
        s.custom,
        [
            Some(U8Op::Set(5)),
            None,
            Some(U8Op::Cycle {
                direction: Direction::Up,
                bounds: None
            })
        ]
    );
    assert_eq!(
        s.checks,
        [Some(BoolOp::Set(true)), None, Some(BoolOp::Toggle)]
    );
    assert_eq!(s.blend_mode, Some(U8Op::Set(7)));
}

#[test]
fn effect_defaults_flag() {
    assert!(only_segment(r#"{"seg":{"fx":3,"fxdef":true}}"#).effect_defaults);
    assert!(only_segment(r#"{"seg":{"fx":3,"fxdef":1}}"#).effect_defaults);
    assert!(!only_segment(r#"{"seg":{"fx":3,"fxdef":"yes"}}"#).effect_defaults);
    assert!(!only_segment(r#"{"seg":{"fx":3,"fxdef":false}}"#).effect_defaults);
    assert!(!only_segment(r#"{"seg":{"fx":3}}"#).effect_defaults);
}

// --- Colours ------------------------------------------------------------------

#[test]
fn colour_arrays() {
    assert_eq!(
        colors("[[255,160,0],[1,2,3,4]]"),
        [rgbw(255, 160, 0, 0), rgbw(1, 2, 3, 4), None]
    );
    assert_eq!(
        colors("[[300,-1,2.7]]")[0],
        rgbw(44, 255, 2, 0),
        "truncated to bytes"
    );
    assert_eq!(
        colors("[[1,2,3,4,5]]")[0],
        rgbw(1, 2, 3, 4),
        "extra channels ignored"
    );
    assert_eq!(colors("[[]]")[0], None, "an empty array changes nothing");
}

#[test]
fn colour_strings() {
    assert_eq!(colors(r#"["00FF00"]"#)[0], rgbw(0, 255, 0, 0));
    assert_eq!(
        colors(r#"["10FF0080"]"#)[0],
        rgbw(0x10, 0xFF, 0x00, 0x80),
        "RRGGBBWW"
    );
    assert_eq!(
        colors(r#"["zz0000"]"#)[0],
        rgbw(0, 0, 0, 0),
        "parsing stops at the first non-hex digit"
    );
    assert_eq!(colors(r#"["F00"]"#)[0], None, "wrong length");
    assert_eq!(colors(r#"["r"]"#)[0], Some(ColorSpec::Random));
}

#[test]
fn colour_numbers() {
    assert_eq!(colors("[2700]")[0], Some(ColorSpec::Kelvin(2700)));
    assert_eq!(colors("[0]")[0], rgbw(0, 0, 0, 0));
    assert_eq!(colors("[-5]")[0], None);
    assert_eq!(colors("[1.5]")[0], None);
    assert_eq!(colors("[true]")[0], None);
    assert_eq!(colors("[null]")[0], None);
}

#[test]
fn colour_objects() {
    assert_eq!(
        colors(r#"[{"g":7,"x":1,"b":300,"g":9}]"#)[0],
        Some(ColorSpec::Partial {
            r: None,
            g: Some(7),
            b: None,
            w: None
        }),
        "first key wins; out-of-range channels keep their value"
    );
}

#[test]
fn colour_slots_beyond_three_and_non_arrays_are_ignored() {
    assert_eq!(colors("[0,0,0,[1,2,3]]"), [rgbw(0, 0, 0, 0); 3]);
    assert_eq!(colors(r#"{"r":1}"#), [None; 3]);
    assert_eq!(colors(r#""FF0000""#), [None; 3]);
}

#[test]
fn hex_reading_matches_strtoul() {
    assert_eq!(strtoul_hex(b"  0x1F"), 0x1F);
    assert_eq!(strtoul_hex(b"ffffffffff"), u32::MAX);
    assert_eq!(strtoul_hex(b"-1"), u32::MAX);
    assert_eq!(strtoul_hex(b""), 0);
}
