//! What a row says: a line's text with or without its timestamp, its level (for the colour), the
//! marker's and the state row's words. Plain Rust, no gpui.

use oxikube_app::logs::{EndReason, LogEntry, LogState};
use oxikube_domain::ErrorKind;

/// How a line is coloured, read from the first words of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `error`, `fatal`, `panic`, `crit`.
    Error,
    /// `warn`, `warning`.
    Warn,
    /// `debug`, `trace`.
    Debug,
    /// Anything else (info, plain output).
    Plain,
}

/// How far into a line the level is looked for: it is in the prefix of every common format
/// (`ERROR ...`, `2026-...Z WARN ...`, `level=error ...`, `{"level":"warn",...`), and a bounded
/// scan keeps a 16 KiB line as cheap as a short one.
const LEVEL_SCAN: usize = 64;

/// The level of `text`: the first level word in its first 64 bytes, case-insensitively, as a
/// whole word (`terror` is not an error).
pub fn level_of(text: &str) -> Level {
    let mut end = text.len().min(LEVEL_SCAN);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    for word in text[..end].split(|c: char| !c.is_ascii_alphabetic()) {
        if word.len() < 4 || word.len() > 7 {
            continue;
        }
        let level = match word.to_ascii_lowercase().as_str() {
            "error" | "fatal" | "panic" | "crit" => Level::Error,
            "warn" | "warning" => Level::Warn,
            "debug" | "trace" => Level::Debug,
            _ => continue,
        };
        return level;
    }
    Level::Plain
}

/// The timestamp column: UTC to the millisecond, fixed width (`2026-10-07T12:00:00.123Z`).
pub fn timestamp(entry: &LogEntry) -> String {
    entry.ts.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The words of the "truncated" marker for `dropped` older lines and a buffer of `capacity`.
pub fn truncated_marker(dropped: u64, capacity: usize) -> String {
    format!(
        "{} older lines dropped: the buffer keeps the newest {} (logs.buffer_lines)",
        group(dropped),
        group(capacity as u64)
    )
}

/// The words of the state row.
pub fn state_text(state: &LogState) -> String {
    match state {
        LogState::Connecting => "Connecting…".to_owned(),
        LogState::Streaming => String::new(),
        LogState::Ended(EndReason::Completed) => "End of log".to_owned(),
        LogState::Ended(EndReason::StreamClosed) => {
            "Stream ended: the container stopped or the connection closed".to_owned()
        }
        LogState::Ended(EndReason::Cancelled) => "Stopped".to_owned(),
        LogState::Failed(failure) => {
            let what = match failure.kind {
                ErrorKind::NotFound => "Not found",
                ErrorKind::Forbidden => "Not allowed to read these logs",
                ErrorKind::Validation => "Cannot read these logs",
                _ if failure.retryable => "Connection failed",
                _ => "Failed",
            };
            format!("{what}: {}", failure.message)
        }
    }
}

/// `12345` as `12,345`.
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxikube_app::logs::LogFailure;

    use super::*;

    #[test]
    fn levels_are_read_from_the_prefix_as_whole_words() {
        assert_eq!(level_of("ERROR something broke"), Level::Error);
        assert_eq!(level_of("2026-10-07T12:00:00Z WARN slow"), Level::Warn);
        assert_eq!(level_of("level=debug msg=hi"), Level::Debug);
        assert_eq!(level_of(r#"{"level":"fatal","msg":"x"}"#), Level::Error);
        assert_eq!(level_of("terror alert"), Level::Plain);
        assert_eq!(level_of("all good"), Level::Plain);
        let late = format!("{} error", "x".repeat(80));
        assert_eq!(level_of(&late), Level::Plain, "only the prefix is scanned");
        assert_eq!(
            level_of("ééééééééééééééééééééééééééééééééééé error"),
            Level::Plain
        );
    }

    #[test]
    fn the_timestamp_is_fixed_width_utc_millis() {
        let entry = LogEntry {
            seq: 0,
            ts: "2026-10-07T12:00:00.123456Z".parse().unwrap(),
            pod: Arc::from("web-0"),
            container: Arc::from("app"),
            text: Arc::from("hi"),
            truncated: false,
            level: None,
        };
        assert_eq!(timestamp(&entry), "2026-10-07T12:00:00.123Z");
    }

    #[test]
    fn marker_and_state_words() {
        assert_eq!(group(1_234_567), "1,234,567");
        assert_eq!(group(12), "12");
        assert!(truncated_marker(1_500, 50_000).starts_with("1,500 older lines dropped"));
        assert_eq!(state_text(&LogState::Connecting), "Connecting…");
        let failed = LogState::Failed(LogFailure {
            kind: ErrorKind::Forbidden,
            message: "pods/log is forbidden".into(),
            retryable: false,
        });
        assert_eq!(
            state_text(&failed),
            "Not allowed to read these logs: pods/log is forbidden"
        );
    }
}
