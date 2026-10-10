//! The root schema of one kind inside an OpenAPI v3 group document (E10-S01).

use super::{JsonSchema, flatten_schema};
use crate::ids::Gvk;

/// Finds the root schema for `gvk` in an OpenAPI v3 group `document` and
/// flattens it with [`flatten_schema`].
///
/// The root is the entry of `components.schemas` whose
/// `x-kubernetes-group-version-kind` list contains `{group, kind, version}`
/// equal to `gvk` (matched by value, never by name guessing). `None` when the
/// document has no such entry.
pub fn root_schema_for(document: &serde_json::Value, gvk: &Gvk) -> Option<JsonSchema> {
    let schemas = document.get("components")?.get("schemas")?.as_object()?;
    let root = schemas.values().find(|schema| {
        schema
            .get("x-kubernetes-group-version-kind")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|gvks| {
                gvks.iter().any(|entry| {
                    entry
                        .get("group")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        == &*gvk.group
                        && entry
                            .get("kind")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            == &*gvk.kind
                        && entry
                            .get("version")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            == &*gvk.version
                })
            })
    })?;
    Some(flatten_schema(root, schemas))
}
