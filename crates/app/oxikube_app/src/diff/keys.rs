//! Per-key comparison of the value maps of a ConfigMap (`data`, `binaryData`) and a Secret
//! (`data`, `stringData`): which keys are the same, changed, or only on one side.
//!
//! The objects are the normalised ones, so a Secret's values are fingerprints here: they decide
//! `same` or `changed`, and nothing in [`KeyDiff`] carries a value.

use serde_json::{Map, Value};

use super::normalize::is_secret;

/// How one key compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    /// Both sides have it, with equal values.
    Same,
    /// Both sides have it, with different values.
    Changed,
    /// Only the left side has it.
    OnlyLeft,
    /// Only the right side has it.
    OnlyRight,
}

impl KeyStatus {
    /// The word the summary shows.
    pub fn label(self) -> &'static str {
        match self {
            KeyStatus::Same => "same",
            KeyStatus::Changed => "changed",
            KeyStatus::OnlyLeft => "only on the left",
            KeyStatus::OnlyRight => "only on the right",
        }
    }
}

/// One key of a ConfigMap or Secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyDiff {
    /// The field the key is in (`data`, `binaryData`, `stringData`).
    pub field: &'static str,
    /// The key.
    pub key: String,
    /// How the two sides compare.
    pub status: KeyStatus,
}

/// The kinds with value maps: ConfigMap and Secret of the core group.
fn fields(value: &Value) -> &'static [&'static str] {
    if is_secret(value) {
        &["data", "stringData"]
    } else if value.get("apiVersion").and_then(Value::as_str) == Some("v1")
        && value.get("kind").and_then(Value::as_str) == Some("ConfigMap")
    {
        &["data", "binaryData"]
    } else {
        &[]
    }
}

/// The keys of the value maps of `left` and `right` (normalised; `Null` for a missing side),
/// sorted by field and key. Empty for any other kind.
pub fn key_report(left: &Value, right: &Value) -> Vec<KeyDiff> {
    let names = match (fields(left), fields(right)) {
        ([], fields) | (fields, _) => fields,
    };
    let empty = Map::new();
    let mut out = Vec::new();
    for field in names {
        let l = left
            .get(*field)
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let r = right
            .get(*field)
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let mut keys: Vec<&String> = l.keys().chain(r.keys()).collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let status = match (l.get(key), r.get(key)) {
                (Some(a), Some(b)) if a == b => KeyStatus::Same,
                (Some(_), Some(_)) => KeyStatus::Changed,
                (Some(_), None) => KeyStatus::OnlyLeft,
                _ => KeyStatus::OnlyRight,
            };
            out.push(KeyDiff {
                field,
                key: key.clone(),
                status,
            });
        }
    }
    out
}
