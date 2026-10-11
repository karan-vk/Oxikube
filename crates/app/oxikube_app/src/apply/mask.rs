//! A Secret's values never leave the apply module: each is replaced by a keyed fingerprint.
//!
//! The dry run returns the object as the server would store it, and the preview compares it with
//! the live one. For a Secret both sides would carry its values; [`SecretMask`] replaces each
//! `data` / `stringData` value with `(hidden <fingerprint>)` before the objects are handed out, so
//! a diff still shows *which* keys change (equal values, equal fingerprints) and never *to what*.
//! The fingerprint is keyed per mask with a random key that never leaves memory, so it cannot be
//! matched against guesses or across previews. The `last-applied-configuration` annotation, which
//! embeds the values, is dropped.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher as _, Hash as _, Hasher as _};

use serde_json::Value;

/// The annotation `kubectl apply` writes: the whole applied manifest, a Secret's data included.
const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// The fields of a Secret that hold values.
const SECRET_FIELDS: [&str; 2] = ["data", "stringData"];

/// What a masked value starts with.
pub const HIDDEN_PREFIX: &str = "(hidden ";

/// Whether `value` is a core `Secret`.
pub fn is_secret(value: &Value) -> bool {
    value.get("apiVersion").and_then(Value::as_str) == Some("v1")
        && value.get("kind").and_then(Value::as_str) == Some("Secret")
}

/// Masks the values of Secrets; one key per mask, so the objects masked by one mask compare.
#[derive(Default)]
pub(crate) struct SecretMask {
    key: RandomState,
}

impl SecretMask {
    /// A mask with a fresh random key.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `value` with its values masked when it is a Secret; any other object as it is.
    pub(crate) fn apply(&self, mut value: Value) -> Value {
        if !is_secret(&value) {
            return value;
        }
        if let Some(annotations) = value
            .pointer_mut("/metadata/annotations")
            .and_then(Value::as_object_mut)
        {
            annotations.shift_remove(LAST_APPLIED);
        }
        for field in SECRET_FIELDS {
            let Some(map) = value.get_mut(field).and_then(Value::as_object_mut) else {
                continue;
            };
            for entry in map.values_mut() {
                *entry = Value::String(self.fingerprint(entry));
            }
        }
        value
    }

    fn fingerprint(&self, value: &Value) -> String {
        let mut hasher = self.key.build_hasher();
        // A string hashes by its text; anything else (a malformed value) by its JSON.
        match value {
            Value::String(text) => text.hash(&mut hasher),
            other => other.to_string().hash(&mut hasher),
        }
        format!("{HIDDEN_PREFIX}{:08x})", hasher.finish() as u32)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_secret_keeps_its_keys_and_loses_its_values() {
        let mask = SecretMask::new();
        let secret = json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "s", "annotations": {LAST_APPLIED: "{\"data\":{\"a\":\"c2VjcmV0\"}}", "keep": "x"}},
            "data": {"a": "c2VjcmV0", "b": "c2VjcmV0"},
            "stringData": {"c": "plain-secret"}
        });
        let masked = mask.apply(secret);
        let text = masked.to_string();
        assert!(
            !text.contains("c2VjcmV0") && !text.contains("plain-secret"),
            "{text}"
        );
        assert!(
            masked
                .pointer(&format!(
                    "/metadata/annotations/{}",
                    LAST_APPLIED.replace('/', "~1")
                ))
                .is_none()
        );
        assert_eq!(masked["metadata"]["annotations"]["keep"], "x");
        // Equal values, equal fingerprints: a diff shows which keys change.
        assert_eq!(masked["data"]["a"], masked["data"]["b"]);
        assert!(
            masked["data"]["a"]
                .as_str()
                .unwrap()
                .starts_with(HIDDEN_PREFIX)
        );
        assert_ne!(masked["data"]["a"], masked["stringData"]["c"]);
    }

    #[test]
    fn other_objects_pass_unchanged_and_keys_differ_per_mask() {
        let config = json!({"apiVersion": "v1", "kind": "ConfigMap", "data": {"a": "b"}});
        assert_eq!(SecretMask::new().apply(config.clone()), config);
        let secret = json!({"apiVersion": "v1", "kind": "Secret", "data": {"a": "c2VjcmV0"}});
        let one = SecretMask::new().apply(secret.clone());
        let two = SecretMask::new().apply(secret);
        assert_ne!(one["data"]["a"], two["data"]["a"], "the key is per mask");
    }
}
