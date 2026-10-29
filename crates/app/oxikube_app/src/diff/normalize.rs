//! What a diff compares: an object with the fields the server owns removed, the ignore list
//! applied, keys sorted and a Secret's values replaced by fingerprints.
//!
//! The editor's diff vs live (`oxikube_editor::diff`) shares the building blocks
//! ([`sort_keys`], [`is_secret`], [`remove_path`]); this module adds the options of a comparison
//! between two clusters or namespaces.

use std::collections::HashMap;
use std::hash::{BuildHasher as _, RandomState};

use parking_lot::Mutex;

use serde_json::{Map, Value};

use super::ignore::IgnorePaths;
use crate::apply::SERVER_OWNED_FIELDS;

/// The fields of a Secret that hold values.
pub const SECRET_FIELDS: [&str; 2] = ["data", "stringData"];

/// The annotation `kubectl apply` writes: the whole applied manifest, a Secret's data included.
pub const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// What a comparison looks at.
#[derive(Clone, Debug, Default)]
pub struct DiffOptions {
    /// Paths to leave out (the `diff.ignore_paths` setting).
    pub ignore: IgnorePaths,
    /// Compare `status` too.
    pub include_status: bool,
    /// Compare `metadata.name` and `metadata.namespace` too.
    pub compare_identity: bool,
}

/// Turns the values of a Secret into labels that are equal exactly when the values are, and say
/// nothing else: `(value #1)`, `(value #2)`, numbered in the order the comparison first meets each
/// distinct value. Values are told apart by a hash keyed at random once per comparison, which is
/// dropped with the [`Fingerprinter`]; no hash of a value, and nothing derived from one beyond
/// "the same as that one", is ever shown or kept. Both sides of a comparison share one.
pub struct Fingerprinter {
    state: RandomState,
    seen: Mutex<HashMap<u64, usize>>,
}

impl Fingerprinter {
    /// A fingerprinter with fresh keys and no values met.
    pub fn new() -> Self {
        Self {
            state: RandomState::new(),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// "(value #1)": tells equal values apart from changed ones without showing either.
    pub fn of(&self, value: &Value) -> String {
        let hash = self.state.hash_one(value.to_string());
        let mut seen = self.seen.lock();
        let next = seen.len() + 1;
        let number = *seen.entry(hash).or_insert(next);
        format!("(value #{number})")
    }
}

impl Default for Fingerprinter {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `value` is a core `Secret` (`apiVersion: v1`, `kind: Secret`).
pub fn is_secret(value: &Value) -> bool {
    value.get("apiVersion").and_then(Value::as_str) == Some("v1")
        && value.get("kind").and_then(Value::as_str) == Some("Secret")
}

/// Removes the node at `path` (keys from the root) from `value`, if there is one.
pub fn remove_path(value: &mut Value, path: &[&str]) {
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

/// `value` as a comparison sees it: the server's fields ([`SERVER_OWNED_FIELDS`], `status`
/// unless asked for) removed, the name and namespace removed unless asked for, the ignore list
/// applied, keys sorted. A Secret's `data` / `stringData` values become fingerprints from
/// `fingerprints` and its last-applied annotation, which embeds them, is dropped.
pub fn normalize(value: &Value, options: &DiffOptions, fingerprints: &Fingerprinter) -> Value {
    let mut out = value.clone();
    for path in SERVER_OWNED_FIELDS {
        if options.include_status && *path == ["status"] {
            continue;
        }
        remove_path(&mut out, path);
    }
    if !options.compare_identity {
        remove_path(&mut out, &["metadata", "name"]);
        remove_path(&mut out, &["metadata", "namespace"]);
    }
    options.ignore.prune(&mut out);
    // An emptied `metadata` reads like none, whichever side lost its fields to the list.
    if out
        .get("metadata")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        remove_path(&mut out, &["metadata"]);
    }
    // Sorted first, so the labels of a Secret's values are numbered in key order.
    let mut out = sort_keys(out);
    if is_secret(value) {
        mask_secret(&mut out, fingerprints);
    }
    out
}

fn mask_secret(value: &mut Value, fingerprints: &Fingerprinter) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for field in SECRET_FIELDS {
        if let Some(map) = object.get_mut(field).and_then(Value::as_object_mut) {
            for entry in map.values_mut() {
                *entry = Value::String(fingerprints.of(entry));
            }
        }
    }
    let Some(meta) = object.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(annotations) = meta.get_mut("annotations").and_then(Value::as_object_mut) {
        annotations.shift_remove(LAST_APPLIED);
        if annotations.is_empty() {
            meta.shift_remove("annotations");
        }
    }
}
