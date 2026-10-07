//! JSON structured-log parsing (E08-S05): pure functions from a line of text to a [`LogRecord`].
//!
//! Most services log JSON, one object per line, and each logger spells the same few fields its
//! own way (zap `ts` as a float, logrus `time` as RFC 3339, bunyan and pino numeric levels 10 to
//! 60 ...). [`parse_line`] reads a line into a normalised level, a time, a message and the
//! remaining fields; [`FieldMap`] is the table of field names it looks for. Detection is per
//! line, so a stream can mix a plain-text banner, JSON lines and half-written JSON: anything
//! that is not a complete JSON *object* is plain text ([`parse_line`] returns `None`).
//!
//! | Piece | Where |
//! |---|---|
//! | [`parse_line`], [`parse_line_with`], [`classify`], [`pretty`] | `mod` |
//! | [`LogRecord`], [`RecordTime`] and the collapsed `key=value` summary | `record` |
//! | [`FieldMap`]: which keys hold the level, time and message | `fields` |
//! | the level spellings (`"warning"`, `"WARN"`, `40`) | [`normalise_level`] (`level`) |
//! | the time spellings (RFC 3339, epoch s / ms / us / ns, floats) | [`parse_time`] (`time`) |
//!
//! # Cost
//!
//! A line is parsed when it is committed to a session's buffer, on the stream's task and before
//! the buffer's lock is taken, and only for its level ([`classify`], stored on the
//! [`LogEntry`](super::LogEntry)); the columns of the rows on screen are parsed again by the view
//! and cached by line. Nothing is parsed per frame. A line that does not start with `{` costs one
//! scan of its leading blanks.
//!
//! # Content is data
//!
//! Parsed text is shown as written and is never written to the application's own logs.

mod fields;
mod level;
mod record;
mod time;

#[cfg(test)]
mod tests;

pub use fields::FieldMap;
pub use level::normalise_level;
pub use record::{LogRecord, RecordTime};
pub use time::parse_time;

use oxikube_domain::log::LogLevel;
use serde_json::{Map, Value};

/// Longest line [`parse_line`] reads, in bytes: a longer one is shown as the raw text it is.
pub const MAX_PARSE_BYTES: usize = 64 * 1024;

/// Whether `text` is worth a parse attempt: it starts with `{` after its leading blanks and is
/// within [`MAX_PARSE_BYTES`].
pub fn looks_like_json(text: &str) -> bool {
    text.len() <= MAX_PARSE_BYTES && text.trim_start().starts_with('{')
}

/// Reads `text` as one structured log record, with the default [`FieldMap`]. `None` when it is
/// not a complete JSON object (plain text, an array, a number, truncated JSON, trailing text).
pub fn parse_line(text: &str) -> Option<LogRecord> {
    parse_line_with(&FieldMap::default(), text)
}

/// [`parse_line`] with the field names in `map`.
pub fn parse_line_with(map: &FieldMap, text: &str) -> Option<LogRecord> {
    Some(LogRecord::from_object(map, parse_object(text)?))
}

/// The level of `text` when it is a structured line: `Some(level)` ([`LogLevel::Unknown`] when
/// it names none), `None` for plain text. `truncated` lines (cut by the per-line cap) are plain
/// text: a cut-off object is not JSON. This is what a session stores per line.
pub fn classify(text: &str, truncated: bool) -> Option<LogLevel> {
    if truncated {
        return None;
    }
    parse_line(text).map(|record| record.level)
}

/// `text` pretty-printed as JSON (two-space indent, fields in source order), `None` when it is
/// not a JSON object. The expanded view of a line.
pub fn pretty(text: &str) -> Option<String> {
    serde_json::to_string_pretty(&parse_object(text)?).ok()
}

/// The JSON object `text` is, when it is [`looks_like_json`] and parses as one.
fn parse_object(text: &str) -> Option<Map<String, Value>> {
    if !looks_like_json(text) {
        return None;
    }
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}
