//! The schema at a JSON path: what hover and completion read.

use oxikube_domain::schema::{AdditionalProperties, JsonSchema};

use crate::yaml::{JsonPath, PathSegment};

/// What the schema says about a position.
pub(super) enum Lookup<'s> {
    /// The schema describes the position.
    Known(&'s JsonSchema),
    /// The schema says nothing: a free-form object, an object that preserves unknown fields, or
    /// a subtree the flattening cut off. No hover, no completion.
    Open,
    /// The schema rules the position out (an unknown field): the validator warns about it.
    Unknown,
}

/// Walks `root` down `path`. Keys follow `properties`, then a schema-valued
/// `additionalProperties` (the value of every key of a map); indexes follow `items`.
pub(super) fn schema_at<'s>(root: &'s JsonSchema, path: &JsonPath) -> Lookup<'s> {
    let mut schema = root;
    for segment in path.segments() {
        if schema.truncated {
            return Lookup::Open;
        }
        schema = match segment {
            PathSegment::Key(key) => {
                if let Some(property) = schema.properties.get(key) {
                    property
                } else if let AdditionalProperties::Schema(each) = &schema.additional_properties {
                    each
                } else if schema.xk8s.preserve_unknown_fields
                    || (schema.properties.is_empty()
                        && matches!(schema.additional_properties, AdditionalProperties::Allowed))
                {
                    return Lookup::Open;
                } else {
                    return Lookup::Unknown;
                }
            }
            PathSegment::Index(_) => match &schema.items {
                Some(items) => items,
                None => return Lookup::Open,
            },
        };
    }
    if schema.truncated {
        Lookup::Open
    } else {
        Lookup::Known(schema)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> JsonSchema {
        JsonSchema::from_value(&json!({
            "type": "object",
            "properties": {
                "spec": {"type": "object", "properties": {
                    "containers": {"type": "array", "items": {"type": "object", "properties": {
                        "image": {"type": "string"}}}},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                    "config": {"type": "object", "x-kubernetes-preserve-unknown-fields": true},
                }}
            }
        }))
    }

    fn path(text: &str) -> JsonPath {
        text.parse().unwrap()
    }

    #[test]
    fn follows_properties_items_and_map_values() {
        let root = schema();
        let known = |p: &str| matches!(schema_at(&root, &path(p)), Lookup::Known(_));
        assert!(known("spec"));
        assert!(known("spec.containers[3].image"));
        assert!(known("spec.labels.app"), "a map's value schema");
        assert!(known("."), "the root");
    }

    #[test]
    fn open_and_unknown_are_told_apart() {
        let root = schema();
        assert!(matches!(
            schema_at(&root, &path("spec.config.anything")),
            Lookup::Open
        ));
        assert!(matches!(
            schema_at(&root, &path("spec.nope")),
            Lookup::Unknown
        ));
        assert!(matches!(
            schema_at(&root, &path("spec.containers[0].nope")),
            Lookup::Unknown
        ));
    }
}
