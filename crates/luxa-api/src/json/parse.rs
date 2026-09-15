//! Parsing state requests.
//!
//! A state request is a JSON object such as
//! `{"on":true,"bri":128,"seg":[{"id":0,"fx":9}],"v":true}`. Parsing is
//! allocation-free and in place — string values are unescaped inside the body
//! buffer — and lenient in the same way as the reference implementation: a
//! value of the wrong type or range is ignored rather than failing the request.
//! Only a body that is not a JSON object (or nests too deeply) is rejected.
//!
//! How each value is read follows the reference implementation's JSON library:
//!
//! - **Level fields** (`bri`, `fx`, `sx`, `ix`, `pal`, `c1`–`c3`, segment
//!   `bri`): an
//!   integer, where one outside `0..=255` becomes `0` and a negative one is
//!   ignored; or a [grammar](crate::grammar) string such as `"~"` or `"r"`.
//! - **Switches** (`on`, `sel`, `rev`, `mi`, `o1`–`o3`): `true`/`false`, or a string
//!   starting with `t` to toggle.
//! - **Other numbers** (`transition`, `tt`, `id`, `start`, `stop`): an integer
//!   that fits the field; anything else is ignored.
//! - **`fxdef`**: `true`, or a nonzero number.
//! - **Duplicate keys**: the first occurrence wins.

use core::fmt;

use luxa_msg::{
    BoolOp, ColorSpec, Command, ErrorCode, GlobalPatch, Name, Rgbw, SegmentPatch, SegmentTarget,
    TransitionTime, U8Op,
};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::grammar::parse_u8_expr;

/// The deepest nesting of arrays and objects a request may use.
///
/// Matches the reference implementation's JSON library, and bounds the
/// parser's recursion on a device with a small stack.
pub const MAX_NESTING: usize = 10;

/// The body could not be read as a state request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl ParseError {
    /// The error code to report to the client.
    pub const fn code(self) -> ErrorCode {
        ErrorCode::Json
    }
}

/// A parsed state request, ready to become commands.
///
/// `SEGMENTS` bounds how many segment entries are kept (extra ones are
/// dropped, as ids past the capacity would be ignored anyway); `NAME` is the
/// byte capacity of a segment name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRequest<const SEGMENTS: usize, const NAME: usize> {
    /// Whether the client asked for the resulting state in the reply (`"v"`).
    pub verbose: bool,
    /// Fixture-wide changes.
    pub global: GlobalPatch,
    segments: [SegmentPatch<NAME>; SEGMENTS],
    segment_count: usize,
    /// For an array of segments: how many entries deleted one.
    deleted: Option<u8>,
    /// Top-level keys in the request, known or not.
    keys: usize,
}

impl<const SEGMENTS: usize, const NAME: usize> StateRequest<SEGMENTS, NAME> {
    /// A request that changes nothing.
    pub const fn new() -> Self {
        Self {
            verbose: false,
            global: GlobalPatch::NONE,
            segments: [SegmentPatch::for_selected(); SEGMENTS],
            segment_count: 0,
            deleted: None,
            keys: 0,
        }
    }

    /// Whether the request is exactly `{"v": true}`: a plain request for the
    /// state, with nothing to apply.
    pub fn is_verbose_only(&self) -> bool {
        self.verbose && self.keys == 1
    }

    /// The segment patches, in request order.
    pub fn segments(&self) -> &[SegmentPatch<NAME>] {
        &self.segments[..self.segment_count]
    }

    /// The request as a run of commands, in the order they must be applied:
    /// fixture-wide changes, each segment, then compaction after an array of
    /// segments.
    pub fn commands(&self) -> impl Iterator<Item = Command<NAME>> + '_ {
        let global = (self.global != GlobalPatch::NONE).then_some(Command::Global(self.global));
        global
            .into_iter()
            .chain(self.segments().iter().copied().map(Command::Segment))
            .chain(
                self.deleted
                    .map(|deleted| Command::CompactSegments { deleted }),
            )
    }

    fn push(&mut self, patch: SegmentPatch<NAME>) {
        if self.segment_count < SEGMENTS {
            self.segments[self.segment_count] = patch;
            self.segment_count += 1;
        }
    }
}

impl<const SEGMENTS: usize, const NAME: usize> Default for StateRequest<SEGMENTS, NAME> {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a state request body, in place.
///
/// Anything after the JSON object is ignored, as the reference
/// implementation's JSON library ignores it.
pub fn parse_state<const SEGMENTS: usize, const NAME: usize>(
    body: &mut [u8],
) -> Result<StateRequest<SEGMENTS, NAME>, ParseError> {
    if nesting(body) > MAX_NESTING {
        return Err(ParseError);
    }
    let mut deserializer = ser_write_json::de::Deserializer::<
        ser_write_json::de::StringByteNopeDecoder,
    >::from_mut_slice(body);
    <StateRequest<SEGMENTS, NAME> as de::Deserialize>::deserialize(&mut deserializer)
        .map_err(|_| ParseError)
}

/// The deepest array/object nesting in `body`, ignoring brackets in strings.
fn nesting(body: &[u8]) -> usize {
    let (mut depth, mut deepest) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for &b in body {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

// --- Values -------------------------------------------------------------------

/// A JSON value reduced to what the field rules look at.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Scalar<'a> {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(&'a str),
    Null,
    /// An array or object, already skipped.
    Composite,
}

impl<'a> Scalar<'a> {
    /// A level field: see the module docs.
    fn level(self) -> Option<U8Op> {
        match self {
            Self::Int(v) if v < 0 => None,
            Self::Int(v) if v <= i64::from(i32::MAX) => {
                Some(U8Op::Set(u8::try_from(v).unwrap_or(0)))
            }
            Self::Str(s) => parse_u8_expr(s),
            _ => None,
        }
    }

    /// A switch: see the module docs.
    fn switch(self) -> Option<BoolOp> {
        match self {
            Self::Bool(b) => Some(BoolOp::Set(b)),
            Self::Str(s) if s.starts_with('t') => Some(BoolOp::Toggle),
            _ => None,
        }
    }

    /// An integer in `min..=max`, or nothing.
    fn fitting(self, min: i64, max: i64) -> Option<i64> {
        match self {
            Self::Int(v) if (min..=max).contains(&v) => Some(v),
            _ => None,
        }
    }

    /// An integer that fits a C `int`.
    fn int(self) -> Option<i64> {
        self.fitting(i64::from(i32::MIN), i64::from(i32::MAX))
    }

    /// A transition in tenths of a second; negative values are ignored.
    fn deciseconds(self) -> Option<TransitionTime> {
        self.int()
            .filter(|v| *v >= 0)
            .map(|v| TransitionTime::from_deciseconds(v.min(i64::from(u16::MAX)) as u16))
    }

    /// A number read as an `int`: integers that fit, floats truncated,
    /// booleans as `0`/`1`; anything else `0`.
    fn as_int(self) -> i32 {
        match self {
            Self::Int(v) => i32::try_from(v).unwrap_or(0),
            Self::Float(f) if f >= f64::from(i32::MIN) && f <= f64::from(i32::MAX) => f as i32,
            Self::Bool(b) => i32::from(b),
            _ => 0,
        }
    }

    /// The value as a C++ `bool` conversion reads it: `true`, or a nonzero
    /// number.
    fn as_bool(self) -> bool {
        match self {
            Self::Bool(b) => b,
            Self::Int(v) => v != 0,
            Self::Float(f) => f != 0.0,
            _ => false,
        }
    }

    /// Whether the value counts as true.
    fn truthy(self) -> bool {
        match self {
            Self::Bool(b) => b,
            Self::Int(v) => v != 0,
            Self::Float(f) => f != 0.0,
            Self::Null => false,
            Self::Str(_) | Self::Composite => true,
        }
    }
}

impl<'de> de::Deserialize<'de> for Scalar<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Scalar<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(Scalar::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(Scalar::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(Scalar::Int(i64::try_from(v).unwrap_or(i64::MAX)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(Scalar::Float(v))
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Scalar::Str(v))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Scalar::Composite)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Scalar::Composite)
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Tracks which keys of an object have been seen, so the first one wins.
#[derive(Default)]
struct Seen(u32);

impl Seen {
    fn first(&mut self, key: u32) -> bool {
        let first = self.0 & key == 0;
        self.0 |= key;
        first
    }
}

// --- The request --------------------------------------------------------------

impl<'de, const SEGMENTS: usize, const NAME: usize> de::Deserialize<'de>
    for StateRequest<SEGMENTS, NAME>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V<const SEGMENTS: usize, const NAME: usize>;
        impl<'de, const SEGMENTS: usize, const NAME: usize> Visitor<'de> for V<SEGMENTS, NAME> {
            type Value = StateRequest<SEGMENTS, NAME>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut request = StateRequest::new();
                let mut seen = Seen::default();
                while let Some(key) = map.next_key::<&str>()? {
                    request.keys += 1;
                    match key {
                        "on" if seen.first(1 << 0) => {
                            request.global.on = map.next_value::<Scalar>()?.switch();
                        }
                        "bri" if seen.first(1 << 1) => {
                            request.global.brightness = map.next_value::<Scalar>()?.level();
                        }
                        "transition" if seen.first(1 << 2) => {
                            request.global.transition = map.next_value::<Scalar>()?.deciseconds();
                        }
                        "tt" if seen.first(1 << 3) => {
                            request.global.transition_once =
                                map.next_value::<Scalar>()?.deciseconds();
                        }
                        "v" if seen.first(1 << 4) => {
                            request.verbose = map.next_value::<Scalar>()? == Scalar::Bool(true);
                        }
                        "seg" if seen.first(1 << 5) => {
                            map.next_value_seed(Segments(&mut request))?;
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(request)
            }
        }
        deserializer.deserialize_map(V::<SEGMENTS, NAME>)
    }
}

// --- Segments -----------------------------------------------------------------

/// The `seg` value: one object, or an array of them.
struct Segments<'r, const SEGMENTS: usize, const NAME: usize>(&'r mut StateRequest<SEGMENTS, NAME>);

impl<'de, const SEGMENTS: usize, const NAME: usize> DeserializeSeed<'de>
    for Segments<'_, SEGMENTS, NAME>
{
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, const SEGMENTS: usize, const NAME: usize> Visitor<'de> for Segments<'_, SEGMENTS, NAME> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a segment object or an array of them")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<(), A::Error> {
        let (mut patch, fields) = read_segment::<A, NAME>(map)?;
        // No id, or a negative one, means every selected segment.
        patch.target = match fields.id {
            Some(id) if id >= 0 => SegmentTarget::Id(id as u8),
            _ => SegmentTarget::Selected,
        };
        self.0.push(patch);
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut index = 0usize;
        let mut deleted = 0u8;
        while seq
            .next_element_seed(Element {
                request: &mut *self.0,
                index,
                deleted: &mut deleted,
            })?
            .is_some()
        {
            index += 1;
        }
        self.0.deleted = Some(deleted);
        Ok(())
    }

    // Anything else leaves the segments alone.
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// One entry of a `seg` array.
struct Element<'r, 'd, const SEGMENTS: usize, const NAME: usize> {
    request: &'r mut StateRequest<SEGMENTS, NAME>,
    index: usize,
    deleted: &'d mut u8,
}

impl<'de, const SEGMENTS: usize, const NAME: usize> DeserializeSeed<'de>
    for Element<'_, '_, SEGMENTS, NAME>
{
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de, const SEGMENTS: usize, const NAME: usize> Visitor<'de>
    for Element<'_, '_, SEGMENTS, NAME>
{
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a segment object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<(), A::Error> {
        let (mut patch, fields) = read_segment::<A, NAME>(map)?;
        // An id that fits a byte names the segment; otherwise the position does.
        let id = fields
            .id
            .and_then(|id| u8::try_from(id).ok())
            .unwrap_or(self.index as u8);
        patch.target = SegmentTarget::Id(id);
        if fields.deletes {
            *self.deleted = self.deleted.saturating_add(1);
        }
        self.request.push(patch);
        Ok(())
    }

    // A non-object entry changes nothing, but still takes its position.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// What a segment object says beyond its patch.
struct SegmentFields {
    id: Option<i64>,
    /// `"stop": 0` — the entry deletes a segment.
    deletes: bool,
}

fn read_segment<'de, A: MapAccess<'de>, const NAME: usize>(
    mut map: A,
) -> Result<(SegmentPatch<NAME>, SegmentFields), A::Error> {
    let mut patch = SegmentPatch::for_selected();
    let mut fields = SegmentFields {
        id: None,
        deletes: false,
    };
    let mut seen = Seen::default();
    while let Some(key) = map.next_key::<&str>()? {
        match key {
            "id" if seen.first(1 << 0) => fields.id = map.next_value::<Scalar>()?.int(),
            "start" if seen.first(1 << 1) => {
                patch.start = map
                    .next_value::<Scalar>()?
                    .fitting(0, i64::from(u16::MAX))
                    .map(|v| v as u16);
            }
            "stop" if seen.first(1 << 2) => {
                let stop = map.next_value::<Scalar>()?;
                fields.deletes = stop == Scalar::Int(0);
                patch.stop = stop.int().filter(|v| *v >= 0).map(|v| v as u16);
            }
            "len" if seen.first(1 << 3) => {
                let len = map.next_value::<Scalar>()?.as_int();
                patch.len = (len > 0).then(|| len.min(i32::from(u16::MAX)) as u16);
            }
            "n" if seen.first(1 << 4) => {
                patch.name = match map.next_value::<Scalar>()? {
                    Scalar::Str(name) => Some(Name::new(name)),
                    // Any other true value clears the name.
                    other if other.truthy() => Some(Name::EMPTY),
                    _ => None,
                };
            }
            "on" if seen.first(1 << 5) => patch.on = map.next_value::<Scalar>()?.switch(),
            "bri" if seen.first(1 << 6) => patch.opacity = map.next_value::<Scalar>()?.level(),
            "col" if seen.first(1 << 7) => map.next_value_seed(Colors(&mut patch.colors))?,
            "fx" if seen.first(1 << 8) => patch.effect = map.next_value::<Scalar>()?.level(),
            "fxdef" if seen.first(1 << 21) => {
                patch.effect_defaults = map.next_value::<Scalar>()?.as_bool();
            }
            "sx" if seen.first(1 << 9) => patch.speed = map.next_value::<Scalar>()?.level(),
            "ix" if seen.first(1 << 10) => patch.intensity = map.next_value::<Scalar>()?.level(),
            "pal" if seen.first(1 << 11) => patch.palette = map.next_value::<Scalar>()?.level(),
            "sel" if seen.first(1 << 12) => patch.selected = map.next_value::<Scalar>()?.switch(),
            "rev" if seen.first(1 << 13) => patch.reverse = map.next_value::<Scalar>()?.switch(),
            "mi" if seen.first(1 << 14) => patch.mirror = map.next_value::<Scalar>()?.switch(),
            "c1" if seen.first(1 << 15) => patch.custom[0] = map.next_value::<Scalar>()?.level(),
            "c2" if seen.first(1 << 16) => patch.custom[1] = map.next_value::<Scalar>()?.level(),
            "c3" if seen.first(1 << 17) => patch.custom[2] = map.next_value::<Scalar>()?.level(),
            "o1" if seen.first(1 << 18) => patch.checks[0] = map.next_value::<Scalar>()?.switch(),
            "o2" if seen.first(1 << 19) => patch.checks[1] = map.next_value::<Scalar>()?.switch(),
            "o3" if seen.first(1 << 20) => patch.checks[2] = map.next_value::<Scalar>()?.switch(),
            _ => {
                map.next_value::<IgnoredAny>()?;
            }
        }
    }
    Ok((patch, fields))
}

// --- Colours ------------------------------------------------------------------

/// The `col` value: an array of up to three colour slots.
struct Colors<'c>(&'c mut [Option<ColorSpec>; 3]);

impl<'de> DeserializeSeed<'de> for Colors<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Colors<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of colours")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut slot = 0;
        while let Some(spec) = seq.next_element_seed(Slot)? {
            if let Some(target) = self.0.get_mut(slot) {
                *target = spec;
            }
            slot += 1;
        }
        Ok(())
    }

    // Anything but an array leaves the colours alone.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// One colour slot: `None` leaves it unchanged.
struct Slot;

impl<'de> DeserializeSeed<'de> for Slot {
    type Value = Option<ColorSpec>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Slot {
    type Value = Option<ColorSpec>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a colour")
    }

    /// `[r, g, b]` or `[r, g, b, w]`: channels a sender left out are zero,
    /// values are truncated to a byte, and an empty array changes nothing.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut channels = [0i32; 4];
        let mut count = 0;
        while let Some(value) = seq.next_element::<Scalar>()? {
            if let Some(channel) = channels.get_mut(count) {
                *channel = value.as_int();
            }
            count += 1;
        }
        let [r, g, b, w] = channels.map(|c| c as u8);
        Ok((count > 0).then_some(ColorSpec::Rgbw(Rgbw::new(r, g, b, w))))
    }

    /// `{"r": .., "g": .., "b": .., "w": ..}`: each channel that is a byte.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut channels = [None; 4];
        let mut seen = Seen::default();
        while let Some(key) = map.next_key::<&str>()? {
            let index = match key {
                "r" => 0,
                "g" => 1,
                "b" => 2,
                "w" => 3,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    continue;
                }
            };
            let value = map.next_value::<Scalar>()?;
            if seen.first(1 << index) {
                channels[index] = value.fitting(0, 255).map(|v| v as u8);
            }
        }
        let [r, g, b, w] = channels;
        Ok(Some(ColorSpec::Partial { r, g, b, w }))
    }

    /// `"RRGGBB"`, `"RRGGBBWW"`, or `"r"` for a random colour.
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(if v == "r" {
            Some(ColorSpec::Random)
        } else {
            hex_color(v).map(ColorSpec::Rgbw)
        })
    }

    /// A colour temperature in kelvin; `0` is black and negative is ignored.
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(kelvin(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(i64::try_from(v).ok().and_then(kelvin))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

fn kelvin(v: i64) -> Option<ColorSpec> {
    match i32::try_from(v) {
        Ok(0) => Some(ColorSpec::Rgbw(Rgbw::BLACK)),
        Ok(k) if k > 0 => Some(ColorSpec::Kelvin(k as u16)),
        _ => None,
    }
}

/// A hex colour string: exactly 6 (`RRGGBB`) or 8 (`RRGGBBWW`) characters,
/// read like C's `strtoul` — hex digits up to the first non-digit.
fn hex_color(text: &str) -> Option<Rgbw> {
    let value = strtoul_hex(text.as_bytes());
    match text.len() {
        6 => Some(Rgbw::new(
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
            0,
        )),
        8 => Some(Rgbw::new(
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        )),
        _ => None,
    }
}

fn strtoul_hex(s: &[u8]) -> u32 {
    let mut s = match s.iter().position(|c| !c.is_ascii_whitespace()) {
        Some(start) => &s[start..],
        None => return 0,
    };
    let negative = match s.first() {
        Some(b'-') => {
            s = &s[1..];
            true
        }
        Some(b'+') => {
            s = &s[1..];
            false
        }
        _ => false,
    };
    if s.len() > 2 && s[0] == b'0' && (s[1] | 0x20) == b'x' && s[2].is_ascii_hexdigit() {
        s = &s[2..];
    }
    let mut value = 0u32;
    for &c in s {
        let Some(digit) = (c as char).to_digit(16) else {
            break;
        };
        value = match value.checked_mul(16).and_then(|v| v.checked_add(digit)) {
            Some(v) => v,
            None => return u32::MAX,
        };
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

#[cfg(test)]
mod tests;
