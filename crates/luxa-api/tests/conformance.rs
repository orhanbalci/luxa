//! The conformance harness: every fixture in `tests/fixtures/api`, run through
//! the protocol, the parser, the real engine and the writers.
//!
//! HTTP fixtures are answered the way `protocol::route` says. WebSocket
//! fixtures play a client against `protocol::ws_frame`, `ws_reply` and
//! `Broadcast`, with time advancing past the cooldown after every frame.

mod support;

use luxa_api::json::{Measure, parse_state, write_error, write_state_and_info, write_success};
use luxa_api::protocol::{
    Broadcast, Document, Frame, Method, Route, WsReply, route, status, ws_frame, ws_reply,
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

fn document(engine: &TestEngine, document: Document) -> Value {
    written(|o| document.write(o, engine.state(), &reference_info(), &ReferenceNames))
}

fn not_implemented() -> (u16, Value) {
    (
        status::NOT_IMPLEMENTED,
        written(|o| write_error(o, ErrorCode::NotImplemented)),
    )
}

/// The response a request produces, answered the way [`route`] says.
fn respond(engine: &mut TestEngine, method: Method, path: &str, body: &[u8]) -> (u16, Value) {
    match route(method, path).expect("fixture paths are under /json") {
        Route::Read(doc) => (status::OK, document(engine, doc)),
        Route::NotImplemented => not_implemented(),
        Route::Apply(doc) => match (post(engine, body), doc) {
            (None, _) => (
                status::BAD_REQUEST,
                written(|o| write_error(o, ErrorCode::Json)),
            ),
            (Some(false), _) => (status::OK, written(write_success)),
            (Some(true), Some(doc)) => (status::OK, document(engine, doc)),
            (Some(true), None) => not_implemented(),
        },
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
            "GET" => respond(&mut engine, Method::Get, path, &[]),
            "POST" => {
                let bytes = match (request.get("body"), request.get("body_raw")) {
                    (Some(body), _) => serde_json::to_vec(body).unwrap(),
                    (None, Some(Value::String(raw))) => raw.as_bytes().to_vec(),
                    _ => panic!("{name}: POST without a body"),
                };
                respond(&mut engine, Method::Post, path, &bytes)
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
            let state = document(&engine, Document::State);
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

/// How a WebSocket client received a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Direct,
    Broadcast,
}

/// Plays a WebSocket client: connects, sends `frames`, and returns every
/// frame it receives in order.
fn play_websocket(engine: &mut TestEngine, frames: &[Value]) -> Vec<(Delivery, String)> {
    let text_of = |engine: &TestEngine, document: Document| {
        let mut out = String::new();
        document
            .write(&mut out, engine.state(), &reference_info(), &ReferenceNames)
            .unwrap();
        out
    };

    let mut broadcast = Broadcast::new();
    let mut now_ms = 0u32;
    let mut received = vec![(Delivery::Direct, text_of(engine, Document::StateAndInfo))];

    for frame in frames {
        let mut bytes = match frame {
            Value::String(text) => text.clone().into_bytes(),
            other => serde_json::to_vec(other).unwrap(),
        };
        match ws_frame::<32, 64>(&mut bytes) {
            Frame::Pong => received.push((Delivery::Direct, "pong".to_owned())),
            Frame::SendState => {
                received.push((Delivery::Direct, text_of(engine, Document::StateAndInfo)));
            }
            Frame::Ignore => {}
            Frame::Apply(request) => {
                let changed = engine
                    .apply_batch(request.commands())
                    .is_some_and(|outcome| outcome.changes.is_state_change());
                if changed {
                    broadcast.changed();
                }
                match ws_reply(request.verbose, broadcast.is_pending()) {
                    Some(WsReply::Success) => {
                        let mut out = String::new();
                        write_success(&mut out).unwrap();
                        received.push((Delivery::Direct, out));
                    }
                    Some(WsReply::StateAndInfo) => {
                        received.push((Delivery::Direct, text_of(engine, Document::StateAndInfo)));
                    }
                    None => {}
                }
            }
        }

        now_ms += 1500;
        if broadcast.due(now_ms) {
            received.push((Delivery::Broadcast, text_of(engine, Document::StateAndInfo)));
        }
    }
    received
}

#[test]
fn every_websocket_fixture_conforms() {
    let mut ran = 0;
    let mut failures = Vec::new();
    for (name, fixture) in fixtures() {
        let request = &fixture["request"];
        if request.get("transport").and_then(Value::as_str) != Some("ws") {
            continue;
        }
        ran += 1;

        let mut engine = engine();
        for before in fixture["before"].as_array().unwrap() {
            post(&mut engine, &serde_json::to_vec(before).unwrap()).expect("setup body parses");
        }
        let received = play_websocket(&mut engine, request["frames"].as_array().unwrap());

        let mut problems = Vec::new();
        let mut frames = received.iter();
        for (index, expected) in fixture["response"]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let delivery = match expected["delivery"].as_str().unwrap() {
                "none" => {
                    if let Some(extra) = frames.next() {
                        problems.push(format!("frame {index}: expected nothing, got {extra:?}"));
                    }
                    continue;
                }
                "direct" => Delivery::Direct,
                "broadcast" => Delivery::Broadcast,
                other => panic!("{name}: unknown delivery {other}"),
            };
            let Some((actual_delivery, text)) = frames.next() else {
                problems.push(format!("frame {index}: nothing received"));
                continue;
            };
            if *actual_delivery != delivery {
                problems.push(format!(
                    "frame {index}: expected {delivery:?}, got {actual_delivery:?}"
                ));
            }
            let result = match expected["match"].as_str().unwrap() {
                "any" => Ok(()),
                "text" => (expected["body"].as_str() == Some(text.as_str()))
                    .then_some(())
                    .ok_or_else(|| format!("text {text:?}")),
                mode => serde_json::from_str::<Value>(text)
                    .map_err(|e| format!("invalid JSON ({e}): {text}"))
                    .and_then(|body| check(&expected["body"], &body, mode)),
            };
            if let Err(e) = result {
                problems.push(format!("frame {index}: {e}"));
            }
        }
        if let Some(extra) = frames.next() {
            problems.push(format!("unexpected frame {extra:?}"));
        }
        if !problems.is_empty() {
            failures.push(format!("{name}:\n    {}", problems.join("\n    ")));
        }
    }
    assert!(ran >= 7, "ran only {ran} fixtures");
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
