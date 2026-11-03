//! Guard: every key context a shipped view sets is a known name, so the validator never warns
//! about a context the app itself uses.

use std::fs;
use std::path::{Path, PathBuf};

use oxikube_keymap::KnownNames;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "tests" | "target" | "snapshots") {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && !name.contains("test") {
            out.push(path);
        }
    }
}

/// Literal names in `.key_context("X")` calls and `const *CONTEXT*: &str = "X"` items.
fn context_names(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let literal = if let Some(rest) = line.split(".key_context(\"").nth(1) {
            rest.split('"').next()
        } else if line.contains("CONTEXT") && line.contains(": &str = \"") {
            line.split(": &str = \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
        } else {
            None
        };
        if let Some(name) = literal.filter(|n| !n.is_empty()) {
            found.push(name.to_owned());
        }
    }
    found
}

#[test]
fn every_context_a_view_sets_is_known() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(!files.is_empty(), "found no ui sources under {crates:?}");
    let known = KnownNames::current();
    let mut missing = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap_or_default();
        for name in context_names(&text) {
            if !known.contains(&name) {
                missing.push(format!("{name} ({})", file.display()));
            }
        }
    }
    assert!(missing.is_empty(), "unknown view contexts: {missing:#?}");
}
