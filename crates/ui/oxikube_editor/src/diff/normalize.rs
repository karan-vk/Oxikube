//! What the diff compares: an object with the fields the server owns removed, its keys sorted,
//! and a Secret's values replaced by fingerprints.
//!
//! [`IGNORED_FIELDS`] is the one list of server-owned fields the diff ignores; the apply preview
//! (E10-S08) and the cross-cluster diff (E20-S09) share it through [`normalize`].

use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use serde_json::{Map, Value};

/// The fields the diff ignores, as paths from the object's root: what the server writes and a
/// manifest never sets (`status`, the bookkeeping under `metadata`). The list is the app's, so the
/// diff and `oxikube_app::apply::Change` (created / configured / unchanged) never disagree.
pub use oxikube_app::apply::SERVER_OWNED_FIELDS as IGNORED_FIELDS;

/// What a masked Secret value reads as in a buffer (the YAML tab's placeholder). The diff treats
/// a key whose value is this text as unchanged: the value was hidden, not edited.
pub const SECRET_PLACEHOLDER: &str = "(hidden)";

/// The annotation `kubectl apply` writes: the whole applied manifest, a Secret's data included.
pub(super) const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// The fields of a Secret that hold values.
pub(super) const SECRET_FIELDS: [&str; 2] = ["data", "stringData"];

/// Whether `value` is a core `Secret` (`apiVersion: v1`, `kind: Secret`).
pub fn is_secret(value: &Value) -> bool {
    value.get("apiVersion").and_then(Value::as_str) == Some("v1")
        && value.get("kind").and_then(Value::as_str) == Some("Secret")
}

/// `value` as the diff compares it: [`IGNORED_FIELDS`] removed and object keys sorted at every
/// level (so key order never shows as a change). For a Secret, each `data` / `stringData` value
/// becomes a fingerprint (equal values, equal fingerprints; the value itself never appears) and
/// the last-applied annotation, which embeds the values, is dropped.
///
/// `reference` is the other side of the diff: a key of this Secret holding
/// [`SECRET_PLACEHOLDER`] takes the reference's fingerprint for the same key, so a masked buffer
/// does not differ from the live Secret it was loaded from.
pub fn normalize(value: &Value, reference: Option<&Value>) -> Value {
    let mut out = value.clone();
    for path in IGNORED_FIELDS {
        remove_path(&mut out, path);
    }
    if is_secret(value) {
        mask_secret(&mut out, reference);
    }
    sort_keys(out)
}

fn remove_path(value: &mut Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut at = value;
    for key in parents {
        match at.get_mut(*key) {
            Some(next) => at = next,
            None => return,
        }
    }
    if let Some(map) = at.as_object_mut() {
        map.shift_remove(*last);
    }
}

fn mask_secret(value: &mut Value, reference: Option<&Value>) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for field in SECRET_FIELDS {
        let Some(map) = object.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for (key, entry) in map.iter_mut() {
            let shown = if entry.as_str() == Some(SECRET_PLACEHOLDER) {
                reference
                    .and_then(|other| other.get(field)?.get(key.as_str()))
                    .filter(|other| other.as_str() != Some(SECRET_PLACEHOLDER))
                    .map_or_else(|| SECRET_PLACEHOLDER.to_owned(), fingerprint)
            } else {
                fingerprint(entry)
            };
            *entry = Value::String(shown);
        }
    }
    let Some(meta) = object.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(annotations) = meta.get_mut("annotations").and_then(Value::as_object_mut) {
        annotations.shift_remove(LAST_APPLIED);
        // Left empty, it would differ from a buffer that has no annotations.
        if annotations.is_empty() {
            meta.shift_remove("annotations");
        }
    }
}

/// "(value #1a2b3c4d)": tells equal values apart from changed ones without showing either.
fn fingerprint(value: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    format!("(value #{:08x})", hasher.finish() as u32)
}

/// `value` with the keys of every object sorted (the workspace's `serde_json` keeps insertion
/// order, so sorting is explicit).
pub fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sort_keys(value)))
                    .collect::<Map<String, Value>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn server_fields_are_removed_and_keys_sorted() {
        let live = json!({
            "kind": "ConfigMap", "apiVersion": "v1",
            "metadata": {"resourceVersion": "9", "name": "a", "uid": "u", "managedFields": []},
            "status": {"x": 1}, "data": {"b": "2", "a": "1"}
        });
        let out = normalize(&live, None);
        assert_eq!(
            out,
            json!({"apiVersion": "v1", "data": {"a": "1", "b": "2"}, "kind": "ConfigMap",
                   "metadata": {"name": "a"}})
        );
        let keys: Vec<_> = out["data"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["a", "b"]);
    }

    #[test]
    fn secret_values_become_fingerprints_and_a_placeholder_matches_the_reference() {
        let live = json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "s", "annotations": {LAST_APPLIED: "{\"data\":{\"k\":\"c2VjcmV0\"}}"}},
            "data": {"k": "c2VjcmV0", "other": "eA=="}
        });
        let mine = json!({
            "apiVersion": "v1", "kind": "Secret", "metadata": {"name": "s"},
            "data": {"k": SECRET_PLACEHOLDER, "other": "eQ=="}
        });
        let live_n = normalize(&live, None);
        let mine_n = normalize(&mine, Some(&live));
        let text = live_n.to_string();
        assert!(!text.contains("c2VjcmV0"), "no value shows: {text}");
        assert_eq!(
            live_n["data"]["k"], mine_n["data"]["k"],
            "masked = unchanged"
        );
        assert_ne!(
            live_n["data"]["other"], mine_n["data"]["other"],
            "an edit shows"
        );
        assert_eq!(
            live_n["metadata"],
            json!({"name": "s"}),
            "last-applied embeds the data"
        );
    }
}
