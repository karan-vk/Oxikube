//! [`prepare`]: the source object turned into the request body for the target.

use oxikube_domain::apply::{SanitizeMode, sanitize_for_apply};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

use crate::apply::manifest::Manifest;

/// The object to write and what was dropped from the source to get it.
pub(super) struct Prepared {
    /// The request body: sanitised, renamed, in the target namespace.
    pub(super) body: Value,
    /// Paths of the fields the sanitiser removed. Never values.
    pub(super) removed: Vec<String>,
}

/// Strips what the server owns from `object` (E20-S02, [`SanitizeMode::Copy`]), then gives it the
/// name and namespace of `target`. A cluster-scoped kind has no namespace.
///
/// # Errors
///
/// A `Validation` error when a namespaced kind has no target namespace, or the object lacks the
/// fields that name it.
pub(super) fn prepare(
    mut object: Value,
    target: &ResourceRef,
    namespaced: bool,
) -> OxiResult<Prepared> {
    let report = sanitize_for_apply(&mut object, SanitizeMode::Copy);
    let namespace = if namespaced {
        Some(target.namespace().ok_or_else(|| {
            OxiError::validation(format!(
                "{} is namespaced: choose the namespace to copy it to",
                target.gvk.kind
            ))
        })?)
    } else {
        None
    };
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.insert("name".into(), Value::String(target.name.to_string()));
        metadata.shift_remove("namespace");
        // A generated name would give the copy another name than the one reviewed.
        metadata.shift_remove("generateName");
    }
    let manifest = Manifest::from_object(object)?;
    Ok(Prepared {
        body: manifest.body(namespace),
        removed: report.removed,
    })
}
