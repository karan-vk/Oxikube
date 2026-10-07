//! Parser corpus (E08-S11): the committed sample logs of zap, logrus, bunyan and pino, plus one
//! mixed stream of plain text and malformed lines, each read line by line and compared with the
//! normalised records committed next to it under `expected/`.
//!
//! The comparison is the contract: a parser change that moves a level, a time, a message or a
//! field shows up as a diff of the expected file. After an intended change, review the diff and
//! rewrite the files with `OXIKUBE_BLESS=1 cargo test -p oxikube_app --test logs_corpus`.
//!
//! The sample logs of the four loggers are shared with the unit tests in `src/logs/parse/tests`,
//! which assert the same lines field by field; this suite pins the whole normalised output.

use std::path::{Path, PathBuf};

use oxikube_app::logs::parse::{LogRecord, parse_line};
use serde_json::{Map, Value, json};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One line as the corpus records it: the normalised record, or the plain text it stayed.
fn normalise(text: &str) -> Value {
    match parse_line(text) {
        Some(record) => record_json(&record),
        None => json!({ "kind": "text", "text": text }),
    }
}

fn record_json(record: &LogRecord) -> Value {
    let time = record.time.as_ref().map(|time| {
        json!({
            // To the microsecond: zap writes epoch seconds as a float, whose rounding below that is
            // an accident of f64, not part of the record.
            "instant": time
                .timestamp
                .and_then(|ts| ts.round(jiff::Unit::Microsecond).ok())
                .map(|ts| ts.to_string()),
            "text": time.text,
            "display": time.display(),
        })
    });
    let fields: Map<String, Value> = record.fields.iter().cloned().collect();
    // `fields` keep the order the logger wrote them; a JSON object would hide a reorder, so the
    // order is recorded apart.
    let order: Vec<&str> = record.fields.iter().map(|(k, _)| k.as_str()).collect();
    json!({
        "kind": "record",
        "level": format!("{:?}", record.level),
        "time": time,
        "message": record.message,
        "fields": fields,
        "field_order": order,
        "summary": record.summary(200),
    })
}

/// Checks `log` (a path under `tests/logs_corpus` or the parser's own fixtures) against
/// `expected/<name>.json`, or writes that file when `OXIKUBE_BLESS` is set.
fn check(name: &str, log: &Path) {
    let text = std::fs::read_to_string(log).unwrap_or_else(|e| panic!("{}: {e}", log.display()));
    let got: Vec<Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(normalise)
        .collect();
    let path = manifest().join(format!("tests/logs_corpus/expected/{name}.json"));
    if std::env::var_os("OXIKUBE_BLESS").is_some() {
        let mut out = serde_json::to_string_pretty(&got).unwrap();
        out.push('\n');
        std::fs::write(&path, out).unwrap();
        return;
    }
    let want: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(got.len(), want.len(), "{name}: line count");
    for (i, (got, want)) in got.iter().zip(&want).enumerate() {
        assert_eq!(
            got,
            want,
            "{name}: record {} differs (bless with OXIKUBE_BLESS=1 if intended)",
            i + 1
        );
    }
}

fn library(name: &str) -> PathBuf {
    manifest().join(format!("src/logs/parse/tests/fixtures/{name}.log"))
}

#[test]
fn zap_lines_normalise_to_the_committed_records() {
    check("zap", &library("zap"));
}

#[test]
fn logrus_lines_normalise_to_the_committed_records() {
    check("logrus", &library("logrus"));
}

#[test]
fn bunyan_lines_normalise_to_the_committed_records() {
    check("bunyan", &library("bunyan"));
}

#[test]
fn pino_lines_normalise_to_the_committed_records() {
    check("pino", &library("pino"));
}

#[test]
fn a_mixed_stream_keeps_malformed_and_plain_lines_as_text() {
    check("mixed", &manifest().join("tests/logs_corpus/mixed.log"));
}

#[test]
fn the_mixed_stream_has_both_kinds_and_every_logger_spelling() {
    // Guards the fixture itself: a blessed run over a emptied or all-text file would pass.
    let text = std::fs::read_to_string(manifest().join("tests/logs_corpus/mixed.log")).unwrap();
    let kinds: Vec<_> = text.lines().map(|l| parse_line(l).is_some()).collect();
    assert!(kinds.iter().filter(|k| **k).count() >= 6);
    assert!(kinds.iter().filter(|k| !**k).count() >= 6);
}
