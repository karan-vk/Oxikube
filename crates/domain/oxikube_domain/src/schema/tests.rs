//! Unit tests for flattening, root lookup and the node types.

use serde_json::json;

use super::*;
use crate::ids::Gvk;

fn deployment_document() -> serde_json::Value {
    json!({
        "components": {
            "schemas": {
                "io.k8s.api.apps.v1.Deployment": {
                    "description": "Deployment enables declarative updates for Pods.",
                    "properties": {
                        "apiVersion": {"type": "string"},
                        "kind": {"type": "string"},
                        "spec": {"$ref": "#/components/schemas/io.k8s.api.apps.v1.DeploymentSpec"}
                    },
                    "required": ["spec"],
                    "type": "object",
                    "x-kubernetes-group-version-kind": [
                        {"group": "apps", "kind": "Deployment", "version": "v1"}
                    ]
                },
                "io.k8s.api.apps.v1.DeploymentSpec": {
                    "properties": {
                        "replicas": {
                            "allOf": [{"$ref": "#/components/schemas/io.k8s.apimachinery.pkg.apis.meta.v1.LabelSelector"}],
                            "description": "stale wrapper kept to test allOf beside a sibling"
                        },
                        "strategy": {"type": "object"}
                    },
                    "type": "object"
                },
                "io.k8s.apimachinery.pkg.apis.meta.v1.LabelSelector": {
                    "properties": {
                        "matchLabels": {
                            "additionalProperties": {"type": "string"},
                            "type": "object"
                        }
                    },
                    "type": "object"
                }
            }
        }
    })
}

#[test]
fn root_lookup_matches_by_gvk_value_not_name() {
    let document = deployment_document();
    let gvk = Gvk::new("apps", "v1", "Deployment");
    let schema = root_schema_for(&document, &gvk).expect("deployment schema");
    assert!(schema.has_property("spec"));
    assert_eq!(schema.required, vec!["spec".to_owned()]);
    assert_eq!(
        schema.description.as_deref(),
        Some("Deployment enables declarative updates for Pods.")
    );
    assert!(root_schema_for(&document, &Gvk::new("apps", "v1", "StatefulSet")).is_none());
}

#[test]
fn all_of_wrapper_resolves_the_reference() {
    let document = deployment_document();
    let schemas = document["components"]["schemas"].as_object().unwrap();
    let spec = flatten_schema(
        &document["components"]["schemas"]["io.k8s.api.apps.v1.DeploymentSpec"],
        schemas,
    );
    let replicas = spec.properties.get("replicas").expect("replicas");
    assert!(
        replicas.has_property("matchLabels"),
        "allOf $ref must resolve to LabelSelector"
    );
    assert_eq!(
        replicas.description.as_deref(),
        Some("stale wrapper kept to test allOf beside a sibling"),
        "the sibling description beside allOf wins"
    );
}

#[test]
fn nested_refs_flatten_transitively() {
    let document = deployment_document();
    let schema = root_schema_for(&document, &Gvk::new("apps", "v1", "Deployment")).expect("schema");
    let spec = schema.properties.get("spec").expect("spec");
    let replicas = spec.properties.get("replicas").expect("replicas");
    let match_labels = replicas.properties.get("matchLabels").expect("matchLabels");
    assert!(matches!(
        match_labels.additional_properties,
        AdditionalProperties::Schema(_)
    ));
}

#[test]
fn recursive_crd_schema_terminates_truncated() {
    let document = json!({
        "components": {
            "schemas": {
                "example.com.v1.Widget": {
                    "properties": {
                        "child": {"$ref": "#/components/schemas/example.com.v1.Widget"}
                    },
                    "type": "object",
                    "x-kubernetes-group-version-kind": [
                        {"group": "example.com", "kind": "Widget", "version": "v1"}
                    ]
                }
            }
        }
    });
    let schema = root_schema_for(&document, &Gvk::new("example.com", "v1", "Widget"))
        .expect("widget schema");
    // One unfolding resolves; the cycle stops at the next level with an
    // open node, so validation never loops.
    let child = schema.properties.get("child").expect("child");
    assert!(!child.truncated, "one unfolding resolves");
    let grandchild = child.properties.get("child").expect("grandchild");
    assert!(
        grandchild.truncated,
        "the recursive reference must stop open"
    );
    assert!(!schema.truncated, "the root itself resolved");
}

#[test]
fn preserve_unknown_fields_marks_the_node_open() {
    let document = json!({
        "components": {
            "schemas": {
                "example.com.v1.Raw": {
                    "type": "object",
                    "x-kubernetes-preserve-unknown-fields": true,
                    "x-kubernetes-group-version-kind": [
                        {"group": "example.com", "kind": "Raw", "version": "v1"}
                    ]
                }
            }
        }
    });
    let schema = root_schema_for(&document, &Gvk::new("example.com", "v1", "Raw")).expect("schema");
    assert!(schema.xk8s.preserve_unknown_fields);
}

#[test]
fn int_or_string_recognised_with_any_of() {
    let document = json!({
        "components": {
            "schemas": {
                "io.k8s.IntOrString": {
                    "anyOf": [{"type": "integer"}, {"type": "string"}],
                    "x-kubernetes-int-or-string": true,
                    "x-kubernetes-group-version-kind": [
                        {"group": "", "kind": "IntOrString", "version": "v1"}
                    ]
                }
            }
        }
    });
    let schema = root_schema_for(&document, &Gvk::new("", "v1", "IntOrString")).expect("schema");
    assert!(schema.xk8s.int_or_string);
    assert!(schema.types.contains(&SchemaType::Integer));
    assert!(schema.types.contains(&SchemaType::String));
}

#[test]
fn missing_reference_stays_open_not_missing() {
    let schemas = serde_json::Map::new();
    let schema = flatten_schema(
        &json!({"$ref": "#/components/schemas/io.k8s.DoesNotExist"}),
        &schemas,
    );
    assert!(schema.truncated);
}

#[test]
fn schema_type_spellings_round_trip() {
    for (text, parsed) in [
        ("object", SchemaType::Object),
        ("array", SchemaType::Array),
        ("integer", SchemaType::Integer),
        ("string", SchemaType::String),
    ] {
        assert_eq!(SchemaType::parse(text), Some(parsed));
        assert_eq!(parsed.as_str(), text);
    }
    assert_eq!(SchemaType::parse("unknown"), None);
}

#[test]
fn properties_are_sorted_searchable_and_merge_with_the_later_winning() {
    let typed = |t| JsonSchema {
        types: vec![t],
        ..JsonSchema::any()
    };
    let mut properties: Properties = [
        ("zeta".to_owned(), typed(SchemaType::String)),
        ("alpha".to_owned(), typed(SchemaType::Integer)),
        ("alpha".to_owned(), typed(SchemaType::Boolean)),
    ]
    .into_iter()
    .collect();
    assert_eq!(properties.keys().collect::<Vec<_>>(), ["alpha", "zeta"]);
    assert_eq!(properties.len(), 2);
    assert_eq!(
        properties.get("alpha").map(|s| s.types.clone()),
        Some(vec![SchemaType::Boolean]),
        "a repeated name keeps the last schema"
    );
    assert!(properties.get("missing").is_none());

    let later: Properties = [("alpha".to_owned(), typed(SchemaType::Number))]
        .into_iter()
        .collect();
    properties.merge(later);
    assert_eq!(
        properties.get("alpha").map(|s| s.types.clone()),
        Some(vec![SchemaType::Number])
    );
    assert!(properties.contains_key("zeta"));
}
