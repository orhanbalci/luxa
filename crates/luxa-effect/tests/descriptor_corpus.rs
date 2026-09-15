//! The descriptor parser, checked against a full corpus of real descriptors.
//!
//! The corpus is not part of this repository. Point `LUXA_DESCRIPTOR_CORPUS`
//! at a C++ source file containing
//! `static const char _data_…[] PROGMEM = "…";` definitions, then run:
//!
//! ```sh
//! LUXA_DESCRIPTOR_CORPUS=path/to/FX.cpp \
//!   cargo test -p luxa-effect --test descriptor_corpus -- --ignored
//! ```

use luxa_effect::Descriptor;

#[test]
#[ignore = "needs a descriptor corpus in LUXA_DESCRIPTOR_CORPUS"]
fn every_descriptor_in_the_corpus_parses_consistently() {
    let path = std::env::var("LUXA_DESCRIPTOR_CORPUS")
        .expect("set LUXA_DESCRIPTOR_CORPUS to a file of descriptor definitions");
    let source = std::fs::read_to_string(&path).expect("corpus is readable");

    const OPEN: &str = "PROGMEM = \"";
    let mut count = 0;
    for line in source.lines().filter(|l| l.contains("_data_")) {
        let Some(start) = line.find(OPEN) else {
            continue;
        };
        let rest = &line[start + OPEN.len()..];
        let Some(end) = rest.rfind("\";") else {
            continue;
        };
        let raw = &rest[..end];
        let d = Descriptor::new(raw);
        count += 1;

        let expected_name = raw.split('@').next().unwrap_or_default();
        assert_eq!(d.name(), expected_name, "{raw}");
        assert!(!d.name().is_empty(), "{raw}");

        if let Some((_, controls)) = raw.split_once('@') {
            // Every `key=value` after the last separator is found, with its value.
            if let Some((_, last)) = controls.rsplit_once(';') {
                for pair in last.split(',').filter(|p| p.contains('=')) {
                    let (key, value) = pair.split_once('=').unwrap();
                    let value: i32 = value.parse().expect("numeric default");
                    assert_eq!(d.default(key), Some(value), "{raw}: {key}");
                }
            }
            // Nothing past the declared slider fields is shown.
            let declared = controls.split(';').next().unwrap_or("").split(',').count();
            assert!(!d.slider(declared.max(8)).is_shown(), "{raw}");
        }

        // The remaining accessors never panic.
        let _ = (d.palette(), d.flags(), d.color(0), d.color(3));
    }
    assert!(count > 100, "found only {count} descriptors in {path}");
}
