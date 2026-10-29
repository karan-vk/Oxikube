//! Structured redaction: the same policy as [`redact`](super::redact), applied to a
//! [`serde_json::Value`] tree (dry-run results, diffs, agent tool output, crash context).

use super::patterns::{MARKER, is_sensitive_field};
use super::scrubber::redact;
use serde_json::Value;
use std::borrow::Cow;

/// Redacts secrets inside `value` in place, keeping keys and structure.
///
/// Two rules apply together, so a Secret that arrives in arbitrary JSON is still covered:
///
/// - **Structural.** In an object whose `kind` is `Secret`, every value under `data`,
///   `stringData` and `secret_data` is replaced (key names stay). A value under any key that
///   [`is_sensitive_field`] accepts (`token`, `password`, `client-key-data`, ...) is replaced
///   whole, whatever its type. A `List` is walked item by item, so Secrets inside it are found; the kind-less items of a
///   `SecretList`, and any kind-less object with a `type` and a `data`/`stringData` body, count
///   as Secrets too.
/// - **Pattern.** Every other string goes through [`redact`] (bearer tokens, JWTs, PEM
///   blocks, URL passwords, an embedded manifest in an annotation, ...).
///
/// Values become [`MARKER`], never a hash of the secret. The function is idempotent, and it
/// leaves `ConfigMap` data alone unless a string pattern matches, so structured output stays
/// useful.
pub fn redact_json(value: &mut Value) {
    redact_walk(value, false);
}

/// Walks `value`; `items_are_secrets` is set for the `items` of a `SecretList`, whose
/// elements carry no `kind` of their own on the wire.
fn redact_walk(value: &mut Value, items_are_secrets: bool) {
    match value {
        Value::String(text) => {
            if let Cow::Owned(clean) = redact(text) {
                *text = clean;
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_walk(item, items_are_secrets);
            }
        }
        Value::Object(map) => {
            let kind = map.get("kind").and_then(Value::as_str);
            let is_secret = kind == Some("Secret")
                || (items_are_secrets && kind.is_none())
                || (kind.is_none() && map.contains_key("type") && has_body_key(map));
            let list_of_secrets = kind == Some("SecretList");
            for (key, child) in map.iter_mut() {
                if is_sensitive_field(key) || (is_secret && is_body_key(key)) {
                    redact_all(child);
                } else {
                    redact_walk(child, list_of_secrets && key == "items");
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Whether the object carries a Secret body key (`data`, `stringData`, ...).
fn has_body_key(map: &serde_json::Map<String, Value>) -> bool {
    map.keys().any(|key| is_body_key(key))
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
