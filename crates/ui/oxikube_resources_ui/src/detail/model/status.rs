//! The `status` summary: the object's `status` flattened to key/value lines.
//!
//! Generic, so it works for any kind including custom resources, and bounded so a hostile or
//! huge status cannot make the drawer slow: at most [`MAX_DEPTH`] levels, [`MAX_LINES`] lines,
//! [`MAX_VALUE`] characters per value, and arrays that are not a few scalars collapse to
//! `[N items]`. `conditions` has its own table and is left out.

use oxikube_domain::Resource;
use serde_json::{Map, Value};

/// Deepest level of nesting expanded (the top level is depth 0).
pub const MAX_DEPTH: u8 = 3;
/// Most lines in the summary.
pub const MAX_LINES: usize = 60;
/// Longest value shown, in characters.
pub const MAX_VALUE: usize = 160;
/// Most scalars an array may hold and still be shown inline.
const INLINE_ARRAY: usize = 5;

/// One line of the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// How deeply the key is nested (0 for a top-level field).
    pub depth: u8,
    /// The field name.
    pub key: String,
    /// The value, or `None` for a field that holds nested fields (its children follow).
    pub value: Option<String>,
}

/// The flattened `status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusSummary {
    /// The lines, in the object's own field order.
    pub lines: Vec<StatusLine>,
    /// Whether lines were left out at [`MAX_LINES`].
    pub truncated: bool,
}

impl StatusSummary {
    /// The summary of `resource`'s `status`; empty when it has none.
    pub fn of(resource: &Resource) -> Self {
        let mut summary = Self::default();
        match resource.get("/status") {
            Some(Value::Object(map)) => summary.walk(map, 0, true),
            Some(Value::Null) | None => {}
            Some(scalar) => {
                summary.push(0, "status", Some(describe(scalar)));
            }
        }
        summary
    }

    fn push(&mut self, depth: u8, key: &str, value: Option<String>) -> bool {
        if self.lines.len() >= MAX_LINES {
            self.truncated = true;
            return false;
        }
        self.lines.push(StatusLine {
            depth,
            key: key.to_owned(),
            value,
        });
        true
    }

    fn walk(&mut self, map: &Map<String, Value>, depth: u8, top: bool) {
        for (key, value) in map {
            if top && key == "conditions" {
                continue;
            }
            let ok = match value {
                Value::Null => true,
                Value::Object(inner) if depth + 1 < MAX_DEPTH && !inner.is_empty() => {
                    if !self.push(depth, key, None) {
                        return;
                    }
                    self.walk(inner, depth + 1, false);
                    true
                }
                other => self.push(depth, key, Some(describe(other))),
            };
            if !ok {
                return;
            }
        }
    }
}

/// A scalar as text, an object as `{N fields}`, an array inline when it is a few scalars and
/// `[N items]` otherwise.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => cut(s, MAX_VALUE),
        Value::Object(map) => format!("{{{} fields}}", map.len()),
        Value::Array(items) => {
            let scalars = items.len() <= INLINE_ARRAY
                && items
                    .iter()
                    .all(|item| !matches!(item, Value::Object(_) | Value::Array(_)));
            if scalars && !items.is_empty() {
                cut(
                    &items.iter().map(describe).collect::<Vec<_>>().join(", "),
                    MAX_VALUE,
                )
            } else {
                format!("[{} items]", items.len())
            }
        }
    }
}

/// `text` cut to `max` characters, with `…` when it was cut.
pub(super) fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}
