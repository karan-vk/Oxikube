//! Structured redaction: the same policy as [`redact`](super::redact), applied to a
//! [`serde_json::Value`] tree (dry-run results, diffs, agent tool output, crash context).

use super::patterns::{MARKER, is_sensitive_field};
use super::scrubber::redact;
use serde_json::Value;

/// Redacts secrets inside `value` in place, keeping keys and structure.
///
/// Two rules apply together, so a Secret that arrives in arbitrary JSON is still covered:
///
/// - **Structural.** In an object whose `kind` is `Secret`, every value under `data`,
///   `stringData` and `secret_data` is replaced (key names stay). A value under any key that
///   [`is_sensitive_field`] accepts (`token`, `password`, `client-key-data`, ...) is replaced
///   whole, whatever its type. A `List` is walked item by item, so Secrets inside it are found.
/// - **Pattern.** Every other string goes through [`redact`] (bearer tokens, JWTs, PEM
///   blocks, URL passwords, an embedded manifest in an annotation, ...).
///
/// Values become [`MARKER`], never a hash of the secret. The function is idempotent, and it
/// leaves `ConfigMap` data alone unless a string pattern matches, so structured output stays
/// useful.
pub fn redact_json(value: &mut Value) {
    walk(value, false);
}

fn walk(value: &mut Value, in_secret_data: bool) {
    match value {
        Value::String(text) => {
            if in_secret_data {
                set_marker(value);
            } else if let std::borrow::Cow::Owned(clean) = redact(text) {
                *text = clean;
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, in_secret_data);
            }
        }
        Value::Object(map) => {
            let is_secret = map.get("kind").and_then(Value::as_str) == Some("Secret");
            for (key, child) in map.iter_mut() {
                if in_secret_data {
                    walk(child, true);
                } else if is_sensitive_field(key) || (is_secret && is_body_key(key)) {
                    redact_all(child);
                } else {
                    walk(child, false);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            if in_secret_data {
                set_marker(value);
            }
        }
    }
}

/// `data` / `stringData` and the snake-case spellings of a Secret.
fn is_body_key(key: &str) -> bool {
    key == "data" || key == "stringData" || key == "string_data" || key == "secret_data"
}

/// Replaces every leaf below `value` with the marker, keeping object keys and array length.
/// `null` is kept: an absent credential is not a secret.
fn redact_all(value: &mut Value) {
    match value {
        Value::Null => {}
        Value::Object(map) => {
            for child in map.values_mut() {
                redact_all(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                redact_all(child);
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Number(_) => set_marker(value),
    }
}

fn set_marker(value: &mut Value) {
    *value = Value::String(MARKER.to_owned());
}
