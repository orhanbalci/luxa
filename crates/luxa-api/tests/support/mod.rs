//! Shared by the integration tests: loading fixtures and matching documents
//! the way `tests/fixtures/api/README.md` specifies.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use luxa_api::json::{Info, Names, Wifi};
use serde_json::Value;

/// Every fixture, as `(file:name, fixture)`.
pub fn fixtures() -> Vec<(String, Value)> {
    let mut all = Vec::new();
    for path in fixture_files() {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for fixture in doc["fixtures"].as_array().unwrap() {
            let name = format!(
                "{}:{}",
                path.file_name().unwrap().to_string_lossy(),
                fixture["name"].as_str().unwrap()
            );
            all.push((name, fixture.clone()));
        }
    }
    assert!(all.len() > 40, "found only {} fixtures", all.len());
    all
}

/// The fixture called `name`, from any file.
pub fn fixture(name: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|(n, _)| n.ends_with(&format!(":{name}")))
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}

fn fixture_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/api");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("fixtures directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
}

/// Checks `actual` against an expected document with a `match` mode.
pub fn check(expected: &Value, actual: &Value, mode: &str) -> Result<(), String> {
    matches(expected, actual, mode == "exact", "$")
}

fn matches(expected: &Value, actual: &Value, exact: bool, path: &str) -> Result<(), String> {
    if let Value::Object(e) = expected
        && e.len() == 1
    {
        if let Some(Value::String(kind)) = e.get("$any") {
            let ok = match kind.as_str() {
                "number" => actual.is_number(),
                "string" => actual.is_string(),
                "boolean" => actual.is_boolean(),
                "object" => actual.is_object(),
                "array" => actual.is_array(),
                other => return Err(format!("{path}: unknown $any {other}")),
            };
            return if ok {
                Ok(())
            } else {
                Err(format!("{path}: expected any {kind}, got {actual}"))
            };
        }
        if let Some(Value::Array(prefix)) = e.get("$prefix") {
            let Value::Array(a) = actual else {
                return Err(format!("{path}: expected an array, got {actual}"));
            };
            if a.len() < prefix.len() {
                return Err(format!("{path}: array shorter than its expected prefix"));
            }
            for (i, (pe, pa)) in prefix.iter().zip(a).enumerate() {
                matches(pe, pa, exact, &format!("{path}[{i}]"))?;
            }
            return Ok(());
        }
    }

    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (key, value) in e {
                let here = format!("{path}.{key}");
                if value == &serde_json::json!({ "$absent": true }) {
                    if a.contains_key(key) {
                        return Err(format!("{here}: must be absent, got {}", a[key]));
                    }
                    continue;
                }
                let Some(actual_value) = a.get(key) else {
                    return Err(format!("{here}: missing"));
                };
                matches(value, actual_value, exact, &here)?;
            }
            if exact {
                if let Some(extra) = a.keys().find(|k| !e.contains_key(*k)) {
                    return Err(format!("{path}.{extra}: not expected"));
                }
            }
            Ok(())
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Err(format!(
                    "{path}: expected {} elements, got {}: {actual}",
                    e.len(),
                    a.len()
                ));
            }
            for (i, (ee, aa)) in e.iter().zip(a).enumerate() {
                matches(ee, aa, exact, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (e, a) if e == a => Ok(()),
        (e, a) => Err(format!("{path}: expected {e}, got {a}")),
    }
}

/// The reference controller's first effect and palette names and its catalogue
/// sizes, for fixtures that check catalogue endpoints.
pub struct ReferenceNames;

const EFFECTS: [&str; 6] = [
    "Solid",
    "Blink",
    "Breathe",
    "Wipe",
    "Wipe Random",
    "Random Colors",
];
const PALETTES: [&str; 7] = [
    "Default",
    "* Random Cycle",
    "* Color 1",
    "* Colors 1&2",
    "* Color Gradient",
    "* Colors Only",
    "Party",
];

impl Names for ReferenceNames {
    fn effect_count(&self) -> u16 {
        220
    }
    fn effect_name(&self, id: u8) -> Option<&str> {
        EFFECTS.get(usize::from(id)).copied().or(Some("Other"))
    }
    fn palette_count(&self) -> u16 {
        72
    }
    fn palette_name(&self, id: u8) -> Option<&str> {
        PALETTES.get(usize::from(id)).copied().or(Some("Other"))
    }
}

/// Device facts matching the fixtures' freshly flashed reference device.
pub fn reference_info() -> Info<'static> {
    Info {
        name: "WLED",
        version: "0.0.0",
        version_id: 0,
        brand: "WLED",
        product: "FOSS",
        arch: "esp32",
        mac: "000000000000",
        ip: "192.168.1.2",
        led_count: 30,
        fps: 42,
        power_ma: 0,
        max_power_ma: 850,
        udp_port: 21324,
        websocket_clients: 0,
        uptime_s: 5,
        free_heap: 100_000,
        options: 0x08,
        wifi: Wifi {
            bssid: "00:00:00:00:00:00",
            rssi: -60,
            signal: 80,
            channel: 6,
            band: "2.4GHz",
            access_point: false,
        },
    }
}
