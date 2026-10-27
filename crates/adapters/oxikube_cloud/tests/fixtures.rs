//! Guards on the recorded CLI output under `tests/fixtures/` (E18-S08): every recording still
//! parses (drift fails loudly, naming the file), and none holds a real account id, tenant id or
//! credential. Re-recording: see `tests/fixtures/README.md`.

use std::fs;
use std::path::{Path, PathBuf};

fn fixture_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("fixtures dir readable") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.file_name().is_some_and(|name| name != "README.md") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    assert!(files.len() >= 20, "fixtures went missing: {files:?}");
    files
}

#[test]
fn every_json_and_yaml_recording_still_parses() {
    for path in fixture_files() {
        let text = fs::read_to_string(&path).expect("fixture is UTF-8");
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => {
                serde_json::from_str::<serde_json::Value>(&text)
                    .unwrap_or_else(|e| panic!("{} no longer parses: {e}", path.display()));
            }
            Some("yaml") => {
                serde_saphyr::from_str::<serde_json::Value>(&text)
                    .unwrap_or_else(|e| panic!("{} no longer parses: {e}", path.display()));
            }
            _ => assert!(!text.trim().is_empty(), "{} is empty", path.display()),
        }
    }
}

/// Digit runs of exactly 12 (an AWS account id), UUIDs (tenant and subscription ids) and
/// well-known credential prefixes.
fn suspicious(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let run = &text[start..i];
            let placeholder = run.bytes().all(|b| b == run.as_bytes()[0]);
            // A run after `-` is a group of a UUID, which is checked below.
            let in_uuid = start > 0 && bytes[start - 1] == b'-';
            if run.len() == 12 && !placeholder && !in_uuid {
                found.push(format!("12-digit id {run}"));
            }
        } else {
            i += 1;
        }
    }
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let groups: Vec<&str> = word.split('-').collect();
        let uuid = groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
            && groups
                .iter()
                .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
        let placeholder = word
            .bytes()
            .filter(|b| *b != b'-')
            .all(|b| matches!(b, b'0' | b'1' | b'2' | b'3'));
        if uuid && !placeholder {
            found.push(format!("uuid {word}"));
        }
        if ["AKIA", "ASIA", "ya29.", "eyJ"]
            .iter()
            .any(|prefix| word.starts_with(prefix))
        {
            found.push(format!("credential-like {word}"));
        }
    }
    found
}

#[test]
fn fixtures_hold_no_real_ids_or_credentials() {
    for path in fixture_files() {
        let text = fs::read_to_string(&path).expect("fixture is UTF-8");
        let found = suspicious(&text);
        assert!(
            found.is_empty(),
            "{} looks un-anonymised: {found:?}",
            path.display()
        );
    }
}

#[test]
fn the_guard_catches_what_it_is_for() {
    assert!(!suspicious("arn:aws:eks:eu-west-1:123456789012:cluster/x").is_empty());
    assert!(!suspicious("tenant 5f0c1c3e-9b1d-4c58-8a77-2d1f3e4a5b6c").is_empty());
    assert!(!suspicious("AKIAIOSFODNN7EXAMPLE").is_empty());
    assert!(suspicious("arn:aws:eks:eu-west-1:111111111111:cluster/x").is_empty());
    assert!(suspicious("00000000-0000-0000-0000-000000000001").is_empty());
}
