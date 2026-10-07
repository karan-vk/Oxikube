//! [`LogRecord`]: one structured line read into its level, time, message and remaining fields.

use jiff::Timestamp;
use jiff::Unit;
use serde_json::{Map, Value};

use oxikube_domain::log::LogLevel;

use super::fields::FieldMap;
use super::level::normalise_level;
use super::time::parse_time;

/// Longest a single value is drawn in the collapsed `key=value` summary, in characters.
const VALUE_CHARS: usize = 80;

/// The time of a record: the instant when it could be read, always the original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTime {
    /// The instant, when the value was an RFC 3339 string or an epoch number.
    pub timestamp: Option<Timestamp>,
    /// The value as the logger wrote it (shown as is when `timestamp` is `None`).
    pub text: String,
}

impl RecordTime {
    /// The column text: `HH:MM:SS.mmm` in UTC when the instant is known (a fixed width, so the
    /// message column lines up), else the original text.
    pub fn display(&self) -> String {
        match self
            .timestamp
            .and_then(|ts| ts.round(Unit::Millisecond).ok())
        {
            Some(ts) => ts.strftime("%H:%M:%S%.3f").to_string(),
            None => self.text.clone(),
        }
    }
}

/// A structured log line: what the columns show and what the expanded view lists.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// The normalised severity ([`LogLevel::Unknown`] when the line names none).
    pub level: LogLevel,
    /// The time the logger wrote, if the line has one.
    pub time: Option<RecordTime>,
    /// The message, if the line has one (a string, or a scalar written as text).
    pub message: Option<String>,
    /// Every other field, in the order the line wrote them.
    pub fields: Vec<(String, Value)>,
}

impl LogRecord {
    /// The record `object` holds, with the field names of `map`. The level, time and message
    /// keys are taken out of the remaining fields.
    pub(super) fn from_object(map: &FieldMap, mut object: Map<String, Value>) -> Self {
        let level = take(&mut object, &map.level, |value| {
            matches!(
                value,
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
            )
        })
        .map_or(LogLevel::Unknown, |value| normalise_level(&value));
        let time = take(&mut object, &map.time, |value| {
            matches!(value, Value::String(_) | Value::Number(_))
        })
        .map(|value| RecordTime {
            timestamp: parse_time(&value),
            text: scalar_text(&value),
        });
        let message = take(&mut object, &map.message, |value| {
            matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
        })
        .map(|value| scalar_text(&value));
        Self {
            level,
            time,
            message,
            fields: object.into_iter().collect(),
        }
    }

    /// The message on one line: line breaks and tabs become spaces, so a row stays one row.
    pub fn message_line(&self) -> String {
        self.message
            .as_deref()
            .map(|message| {
                message
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The collapsed form of the remaining fields: `key=value key2="a value"`, at most `max`
    /// characters (a longer one ends in `…`). Strings without blanks are bare, nested values are
    /// compact JSON.
    pub fn summary(&self, max: usize) -> String {
        let mut out = String::new();
        let mut chars = 0usize;
        let mut cut_short = false;
        for (key, value) in &self.fields {
            if chars >= max {
                cut_short = true;
                break;
            }
            if !out.is_empty() {
                out.push(' ');
                chars += 1;
            }
            let piece = format!("{key}={}", short_value(value));
            chars += piece.chars().count();
            out.push_str(&piece);
        }
        if cut_short || chars > max {
            let cut: String = out.chars().take(max.saturating_sub(1)).collect();
            return format!("{cut}…");
        }
        out
    }
}

/// Removes and returns the value of the first of `keys` present (case-insensitively) with a
/// value `accept` takes; a key whose value does not fit is left in the fields.
fn take(
    object: &mut Map<String, Value>,
    keys: &[String],
    accept: impl Fn(&Value) -> bool,
) -> Option<Value> {
    for wanted in keys {
        let found = object
            .iter()
            .find(|(key, value)| key.eq_ignore_ascii_case(wanted) && accept(value))
            .map(|(key, _)| key.clone());
        if let Some(key) = found {
            return object.shift_remove(&key);
        }
    }
    None
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn short_value(value: &Value) -> String {
    let text = match value {
        Value::String(text) if text.is_empty() || text.contains(char::is_whitespace) => {
            serde_json::to_string(text).unwrap_or_default()
        }
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() <= VALUE_CHARS {
        return text;
    }
    let cut: String = text.chars().take(VALUE_CHARS - 1).collect();
    format!("{cut}…")
}
