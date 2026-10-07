//! [`LogLine`]: one line of container output, as it flows from the log
//! adapter to the logs view and to agent context.
//!
//! Log streams run at up to 5 000 lines/s (`docs/PERFORMANCE.md`), so the
//! record is small: `pod` and `container` are `Arc<str>` (cloned per line
//! without allocating) and the only per-line allocation is the text itself.
//!
//! # Bounds
//!
//! * The text is capped at [`MAX_LOG_LINE_BYTES`], cut on a char boundary. A
//!   cut sets [`LogLine::truncated`], and the flag survives a serde round-trip.
//!   [`LogLine::new`] enforces the cap and so does deserialisation, so a
//!   stored line can never exceed it.
//! * JSON is parsed lazily. [`LogLine::new`] never parses; call
//!   [`LogLine::parse_json`] for lines the UI actually shows. Only text whose
//!   first non-blank character is `{` is attempted, and truncated lines are
//!   never parsed (a cut-off object is not valid JSON anyway).
//!
//! Like every record in this crate, a `LogLine` is not redacted here. The log
//! adapter redacts before building one (non-negotiable 5).

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::bounds::truncate_in_place;

mod level;
mod range;
mod save;

pub use level::{LevelChip, LogLevel};
pub use range::LogRange;
pub use save::LogSaveScope;

/// Longest log line text kept, in bytes (16 KiB). Longer lines are cut on a
/// char boundary and flagged [`LogLine::truncated`].
pub const MAX_LOG_LINE_BYTES: usize = 16 * 1024;

/// One line of container output.
///
/// Field names are stable: the type is persisted and shown to agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "LogLineWire")]
pub struct LogLine {
    /// When the line was written (the kubelet timestamp prefix).
    pub ts: Timestamp,
    /// Pod name.
    pub pod: Arc<str>,
    /// Container name within the pod.
    pub container: Arc<str>,
    /// The raw line without the trailing newline, at most [`MAX_LOG_LINE_BYTES`] long.
    pub text: String,
    /// Whether `text` was cut to fit [`MAX_LOG_LINE_BYTES`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// The line parsed as JSON; `None` until [`LogLine::parse_json`] succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<serde_json::Value>,
}

/// Deserialisation shape; converting through it re-applies the size cap.
#[derive(Deserialize)]
struct LogLineWire {
    ts: Timestamp,
    pod: Arc<str>,
    container: Arc<str>,
    text: String,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    parsed: Option<serde_json::Value>,
}

impl From<LogLineWire> for LogLine {
    fn from(w: LogLineWire) -> Self {
        let mut line = LogLine::new(w.ts, w.pod, w.container, w.text);
        line.truncated |= w.truncated;
        line.parsed = w.parsed;
        line
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl LogLine {
    /// Build a line, cutting `text` to [`MAX_LOG_LINE_BYTES`] on a char boundary
    /// and setting [`truncated`](Self::truncated) when it was cut. Does not parse JSON.
    pub fn new(
        ts: Timestamp,
        pod: impl Into<Arc<str>>,
        container: impl Into<Arc<str>>,
        text: impl Into<String>,
    ) -> Self {
        let mut text = text.into();
        let truncated = truncate_in_place(&mut text, MAX_LOG_LINE_BYTES);
        Self {
            ts,
            pod: pod.into(),
            container: container.into(),
            text,
            truncated,
            parsed: None,
        }
    }

    /// Whether the text looks like a JSON object and is worth a parse attempt.
    ///
    /// A cheap check on the first non-blank byte; it does not validate.
    pub fn looks_like_json(&self) -> bool {
        !self.truncated && self.text.trim_start().starts_with('{')
    }

    /// Parse the text as a JSON object into [`parsed`](Self::parsed).
    ///
    /// Returns `true` when `parsed` is set afterwards. Lines that are not
    /// objects, are truncated, or fail to parse leave `parsed` as `None`.
    /// Idempotent: an already parsed line is not parsed again.
    pub fn parse_json(&mut self) -> bool {
        if self.parsed.is_none() && self.looks_like_json() {
            self.parsed = serde_json::from_str::<serde_json::Value>(&self.text)
                .ok()
                .filter(serde_json::Value::is_object);
        }
        self.parsed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ts() -> Timestamp {
        "2026-10-03T12:00:00.123456789Z".parse().unwrap()
    }

    fn line(text: &str) -> LogLine {
        LogLine::new(ts(), "web-0", "app", text)
    }

    #[test]
    fn short_line_is_untouched() {
        let l = line("hello");
        assert_eq!(l.text, "hello");
        assert!(!l.truncated);
        assert!(l.parsed.is_none());
    }

    #[test]
    fn long_line_is_cut_and_flagged() {
        let l = line(&"a".repeat(MAX_LOG_LINE_BYTES + 100));
        assert_eq!(l.text.len(), MAX_LOG_LINE_BYTES);
        assert!(l.truncated);
    }

    #[test]
    fn exact_cap_is_not_truncated() {
        let l = line(&"a".repeat(MAX_LOG_LINE_BYTES));
        assert!(!l.truncated);
    }

    #[test]
    fn cut_is_on_char_boundary() {
        // 'é' is 2 bytes, so the cap falls inside a char when the prefix is odd.
        let text = format!("x{}", "é".repeat(MAX_LOG_LINE_BYTES));
        let l = line(&text);
        assert!(l.truncated);
        assert!(l.text.len() <= MAX_LOG_LINE_BYTES);
        // 1 + 2 * 8191 bytes fit; the next 'é' would straddle the cap.
        assert_eq!(l.text.len(), MAX_LOG_LINE_BYTES - 1);
    }

    #[test]
    fn json_object_is_parsed_lazily() {
        let mut l = line(r#"  {"level":"info","msg":"ok"}"#);
        assert!(l.parsed.is_none(), "new() must not parse");
        assert!(l.parse_json());
        assert_eq!(l.parsed.as_ref().unwrap()["level"], "info");
        assert!(l.parse_json(), "idempotent");
    }

    #[test]
    fn non_json_lines_are_not_parsed() {
        for text in ["plain text", "[1,2,3]", "{not json", "", "   "] {
            let mut l = line(text);
            assert!(!l.parse_json(), "{text:?}");
            assert!(l.parsed.is_none());
        }
    }

    #[test]
    fn truncated_json_is_not_parsed() {
        let body = "a".repeat(MAX_LOG_LINE_BYTES);
        let mut l = line(&format!(r#"{{"k":"{body}"}}"#));
        assert!(l.truncated);
        assert!(!l.looks_like_json());
        assert!(!l.parse_json());
    }

    #[test]
    fn serde_round_trip_without_optionals() {
        let l = line("hello");
        let json = serde_json::to_value(&l).unwrap();
        assert!(json.get("truncated").is_none());
        assert!(json.get("parsed").is_none());
        assert_eq!(json["pod"], "web-0");
        assert_eq!(json["container"], "app");
        assert_eq!(json["text"], "hello");
        let back: LogLine = serde_json::from_value(json).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn serde_round_trip_with_optionals() {
        let mut l = line(r#"{"a":1}"#);
        l.parse_json();
        l.truncated = true;
        let s = serde_json::to_string(&l).unwrap();
        let back: LogLine = serde_json::from_str(&s).unwrap();
        assert_eq!(back, l);
        assert!(back.truncated);
        assert!(back.parsed.is_some());
    }

    #[test]
    fn serde_round_trip_preserves_truncation_flag() {
        let l = line(&"a".repeat(MAX_LOG_LINE_BYTES + 1));
        let back: LogLine = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert!(back.truncated);
        assert_eq!(back, l);
    }

    #[test]
    fn deserialising_an_oversize_line_re_applies_the_cap() {
        let json = serde_json::json!({
            "ts": "2026-10-03T12:00:00Z",
            "pod": "p",
            "container": "c",
            "text": "a".repeat(MAX_LOG_LINE_BYTES * 2),
        });
        let l: LogLine = serde_json::from_value(json).unwrap();
        assert_eq!(l.text.len(), MAX_LOG_LINE_BYTES);
        assert!(l.truncated);
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics(text in any::<String>()) {
            let mut l = LogLine::new(ts(), "p", "c", text.clone());
            prop_assert!(l.text.len() <= MAX_LOG_LINE_BYTES);
            prop_assert!(text.starts_with(&l.text));
            prop_assert_eq!(l.truncated, text.len() > MAX_LOG_LINE_BYTES);
            l.parse_json();
        }

        #[test]
        fn arbitrary_multibyte_text_cuts_cleanly(s in "[é😀a]{0,12000}") {
            let l = LogLine::new(ts(), "p", "c", s.clone());
            prop_assert!(l.text.len() <= MAX_LOG_LINE_BYTES);
            prop_assert!(s.starts_with(&l.text));
            // At most 3 bytes can be dropped to reach a boundary.
            if l.truncated {
                prop_assert!(l.text.len() + 4 > MAX_LOG_LINE_BYTES);
            }
        }
    }
}
