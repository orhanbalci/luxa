//! The conformance harness: every HTTP fixture in `tests/fixtures/api`, run
//! through the parser, the real engine and the writers.
//!
//! WebSocket fixtures are checked once the WebSocket protocol exists.

mod support;

use luxa_api::json::{
    Measure, parse_state, write_effect_names, write_error, write_everything, write_info,
    write_palette_names, write_state, write_state_and_info, write_success,
};
use luxa_core::Engine;
use luxa_msg::{Catalogue, ErrorCode, Layout, Name, Segment, State};
use serde_json::Value;
use support::{ReferenceNames, check, fixtures, reference_info};

type TestEngine = Engine<32, 64>;

/// The fixtures' device: 30 RGB LEDs and the reference catalogue sizes.
fn engine() -> TestEngine {
    TestEngine::new(Layout::new(30), Catalogue::contiguous(220, 72))
}

/// Applies a request body the way a POST does, returning whether it parsed and
/// whether it asked for the resulting state.
fn post(engine: &mut TestEngine, body: &[u8]) -> Option<bool> {
    let mut bytes = body.to_vec();
    let request = parse_state::<32, 64>(&mut bytes).ok()?;
    engine.apply_batch(request.commands());
    Some(request.verbose)
}

fn written(f: impl FnOnce(&mut String) -> std::fmt::Result) -> Value {
    let mut out = String::new();
    f(&mut out).unwrap();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("invalid JSON ({e}): {out}"))
}

/// The response a GET of `path` produces.
fn get(engine: &TestEngine, path: &str) -> (u16, Value) {
    let state = engine.state();
    let info = reference_info();
    let names = ReferenceNames;
    match path {
        "/json" => (200, written(|o| write_everything(o, state, &info, &names))),
        "/json/state" => (200, written(|o| write_state(o, state))),
        "/json/si" => (
            200,
            written(|o| write_state_and_info(o, state, &info, &names)),
        ),
        "/json/info" => (200, written(|o| write_info(o, &info, state, &names))),
        "/json/eff" => (200, written(|o| write_effect_names(o, &names))),
        "/json/pal" => (200, written(|o| write_palette_names(o, &names))),
        _ => (501, written(|o| write_error(o, ErrorCode::NotImplemented))),
    }
}

/// The response a POST of `body` to `path` produces.
fn respond_to_post(engine: &mut TestEngine, path: &str, body: &[u8]) -> (u16, Value) {
    match post(engine, body) {
        None => (400, written(|o| write_error(o, ErrorCode::Json))),
        Some(true) => get(engine, path),
        Some(false) => (200, written(write_success)),
    }
}

#[test]
fn every_http_fixture_conforms() {
    let mut ran = 0;
    let mut failures = Vec::new();
    for (name, fixture) in fixtures() {
        let request = &fixture["request"];
        if request.get("transport").and_then(Value::as_str) == Some("ws") {
            continue;
        }

        let mut engine = engine();
        for before in fixture["before"].as_array().unwrap() {
            post(&mut engine, &serde_json::to_vec(before).unwrap()).expect("setup body parses");
        }

        let path = request["path"].as_str().unwrap();
        let (status, body) = match request["method"].as_str().unwrap() {
            "GET" => get(&engine, path),
            "POST" => {
                let bytes = match (request.get("body"), request.get("body_raw")) {
                    (Some(body), _) => serde_json::to_vec(body).unwrap(),
                    (None, Some(Value::String(raw))) => raw.as_bytes().to_vec(),
                    _ => panic!("{name}: POST without a body"),
                };
                respond_to_post(&mut engine, path, &bytes)
            }
            other => panic!("{name}: unsupported method {other}"),
        };
        ran += 1;

        let expected = &fixture["response"];
        let mut problems = Vec::new();
        if expected["status"].as_u64() != Some(u64::from(status)) {
            problems.push(format!(
                "status: expected {}, got {status}",
                expected["status"]
            ));
        }
        if let Err(e) = check(
            &expected["body"],
            &body,
            expected["match"].as_str().unwrap(),
        ) {
            problems.push(format!("response {e}"));
        }
        if let Some(after) = fixture.get("after") {
            let (_, state) = get(&engine, "/json/state");
            if let Err(e) = check(&after["body"], &state, after["match"].as_str().unwrap()) {
                problems.push(format!("after {e}"));
            }
        }
        if !problems.is_empty() {
            failures.push(format!("{name}:\n    {}", problems.join("\n    ")));
        }
    }
    assert!(ran > 35, "ran only {ran} fixtures");
    assert!(
        failures.is_empty(),
        "{} of {ran} fixtures failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Step 2's budget: the largest state and info document this firmware can
/// produce must fit a fixed response buffer.
#[test]
fn a_full_capacity_state_fits_the_response_budget() {
    const BUDGET: usize = 24 * 1024;

    let mut state = State::<32, 64>::new(Layout::new(1000));
    // A worst case: every segment active, long numbers, and a full name made
    // of characters that must be escaped.
    let escaped_name = Name::new(&"\"".repeat(64));
    for i in 1..32 {
        state
            .push_segment(Segment::new(i * 30, i * 30 + 29))
            .unwrap();
    }
    for segment in state.segments_mut() {
        segment.name = escaped_name;
        segment.colors = [luxa_msg::Rgbw::new(255, 255, 255, 255); 3];
    }

    let mut measure = Measure::default();
    write_state_and_info(&mut measure, &state, &reference_info(), &ReferenceNames).unwrap();
    eprintln!("full-capacity /json/si: {} bytes", measure.0);
    assert!(
        measure.0 <= BUDGET,
        "a full-capacity /json/si is {} bytes, over the {BUDGET}-byte budget",
        measure.0
    );
}
