//! Parsing real request bodies: every request in the fixtures, and a stream of
//! mutated bodies that must never make the parser panic.

use std::path::Path;

use luxa_api::json::{StateRequest, parse_state};

type Request = StateRequest<32, 64>;

fn fixtures() -> Vec<(String, serde_json::Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/api");
    let mut all = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("fixtures directory") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let doc: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for fixture in doc["fixtures"].as_array().unwrap() {
                let name = format!(
                    "{}:{}",
                    path.file_name().unwrap().to_string_lossy(),
                    fixture["name"].as_str().unwrap()
                );
                all.push((name, fixture.clone()));
            }
        }
    }
    assert!(all.len() > 40, "found only {} fixtures", all.len());
    all
}

fn parse(bytes: &[u8]) -> Result<Request, luxa_api::json::ParseError> {
    let mut body = bytes.to_vec();
    parse_state(&mut body)
}

#[test]
fn every_fixture_request_body_parses() {
    let mut bodies = 0;
    for (name, fixture) in fixtures() {
        for before in fixture["before"].as_array().unwrap() {
            parse(&serde_json::to_vec(before).unwrap())
                .unwrap_or_else(|e| panic!("{name} before: {e:?}"));
            bodies += 1;
        }
        let request = &fixture["request"];
        if let Some(body) = request.get("body") {
            parse(&serde_json::to_vec(body).unwrap()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            bodies += 1;
        }
        if let Some(raw) = request.get("body_raw").and_then(|r| r.as_str()) {
            assert!(
                parse(raw.as_bytes()).is_err(),
                "{name}: malformed body must fail"
            );
        }
        for frame in request
            .get("frames")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
        {
            match frame {
                serde_json::Value::Object(_) => {
                    parse(&serde_json::to_vec(frame).unwrap())
                        .unwrap_or_else(|e| panic!("{name} frame: {e:?}"));
                    bodies += 1;
                }
                serde_json::Value::String(text) if text != "p" => {
                    assert!(
                        parse(text.as_bytes()).is_err(),
                        "{name}: malformed frame must fail"
                    );
                }
                _ => {}
            }
        }
    }
    assert!(bodies > 40, "parsed only {bodies} bodies");
}

#[test]
fn the_reference_ui_power_toggle_body_parses() {
    let r = parse(
        br#"{"on":true,"live":false,"seg":[{"id":0,"frz":false}],"v":true,"time":1757923200}"#,
    )
    .unwrap();
    assert!(r.verbose);
    assert_eq!(r.segments().len(), 1);
}

/// A dependency-free fuzz: deterministic mutations of real bodies. The parser
/// may reject them; it must never panic.
#[test]
fn mutated_bodies_never_panic() {
    let mut seeds: Vec<Vec<u8>> = fixtures()
        .iter()
        .filter_map(|(_, f)| f["request"].get("body"))
        .map(|b| serde_json::to_vec(b).unwrap())
        .collect();
    seeds.push(
        br#"{"seg":[{"id":1,"col":[[1,2,3],"FF00AA",{"r":1},2700],"n":"a\"b"}],"bri":"4~8r"}"#
            .to_vec(),
    );

    const ALPHABET: &[u8] = br#"{}[]",:\ 0123456789-+.eEtrufalsenwx~ABCDEF"#;
    let mut state = 0x9E37_79B9u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as usize
    };

    let mut rejected = 0;
    for round in 0..20_000 {
        let mut body = seeds[round % seeds.len()].clone();
        for _ in 0..1 + next() % 4 {
            if body.is_empty() {
                break;
            }
            let at = next() % body.len();
            match next() % 4 {
                0 => body[at] = ALPHABET[next() % ALPHABET.len()],
                1 => {
                    body.remove(at);
                }
                2 => body.insert(at, ALPHABET[next() % ALPHABET.len()]),
                _ => body.truncate(at),
            }
        }
        if parse(&body).is_err() {
            rejected += 1;
        }
    }
    assert!(
        rejected > 0 && rejected < 20_000,
        "mutations should both pass and fail: {rejected}"
    );
}
