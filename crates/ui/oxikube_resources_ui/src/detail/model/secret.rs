//! What the detail may show of a Secret: its key names, never a value (non-negotiable 5).
//!
//! The store watches Secrets metadata-only, so the object it holds has no data. The detail reads
//! the full object once to list the keys, and [`mask_secret`] removes every value from it before
//! it leaves the read, so values never reach the view. The model also reads only key names from
//! whatever object it is given, so a Secret that did arrive whole would still show no value.
//! Masking and reveal rules proper are E07-S06 and E12.

use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;

/// What stands in for a value that is not shown.
pub const HIDDEN: &str = "(hidden)";

/// The annotation `kubectl apply` writes: the whole applied manifest, a Secret's data included.
const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";

/// Whether `gvk` is the core `Secret`.
pub fn is_secret(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "Secret"
}

/// Whether the annotation `key` carries a copy of the object's data.
pub(super) fn embeds_values(key: &str) -> bool {
    key == LAST_APPLIED
}

/// The names of the keys under `data` and `stringData`, sorted, without a value.
pub(super) fn keys(resource: &Resource) -> Vec<String> {
    let mut keys: Vec<String> = ["data", "stringData"]
        .into_iter()
        .filter_map(|field| resource.json.get(field)?.as_object())
        .flat_map(|map| map.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Removes every secret value from `resource` (`data`, `stringData`, the last-applied
/// annotation) and returns its key names. A resource that is not a Secret is left alone and
/// gives no keys.
pub fn mask_secret(resource: &mut Resource) -> Option<Vec<String>> {
    mask_secret_with(resource, None)
}

/// [`mask_secret`] with `placeholder` (a string) standing where each value was, instead of
/// `null`: the YAML tab shows `key: (hidden)` rather than a bare `null` that reads as an empty
/// value.
pub fn mask_secret_with(resource: &mut Resource, placeholder: Option<&str>) -> Option<Vec<String>> {
    if !is_secret(&resource.kind) {
        return None;
    }
    let names = keys(resource);
    if let Some(object) = resource.json.as_object_mut() {
        for field in ["data", "stringData"] {
            if let Some(map) = object.get_mut(field).and_then(|v| v.as_object_mut()) {
                for value in map.values_mut() {
                    *value = placeholder.map_or(serde_json::Value::Null, |text| text.into());
                }
            }
        }
        if let Some(annotations) = object
            .get_mut("metadata")
            .and_then(|m| m.get_mut("annotations"))
            .and_then(|a| a.as_object_mut())
        {
            annotations.remove(LAST_APPLIED);
        }
    }
    resource
        .meta
        .annotations
        .retain(|key, _| &**key != LAST_APPLIED);
    Some(names)
}
