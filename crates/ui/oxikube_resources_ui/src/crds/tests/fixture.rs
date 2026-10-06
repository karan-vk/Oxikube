//! A CRD as the API server stores it, for the tests of this module and of the views over it.

use oxikube_domain::Resource;
use serde_json::{Value, json};

/// The CRD `widgets.example.com`: namespaced, short name `wd`, category `all`, three versions
/// (`v1` storage, `v1beta1` served, `v1alpha1` not served), the schema of `v1` rich enough to
/// show every kind of row, and printer columns on `v1`.
pub(crate) fn widget_crd_json() -> Value {
    json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {
            "name": "widgets.example.com",
            "uid": "crd-uid",
            "resourceVersion": "5",
            "creationTimestamp": "2026-01-01T00:00:00Z"
        },
        "spec": {
            "group": "example.com",
            "scope": "Namespaced",
            "names": {
                "plural": "widgets", "singular": "widget", "kind": "Widget",
                "listKind": "WidgetList", "shortNames": ["wd", "wdg"], "categories": ["all"]
            },
            "versions": [
                {
                    "name": "v1alpha1", "served": false, "storage": false,
                    "schema": {"openAPIV3Schema": {"type": "object"}}
                },
                {
                    "name": "v1beta1", "served": true, "storage": false, "deprecated": true,
                    "schema": {"openAPIV3Schema": {"type": "object", "properties": {
                        "spec": {"type": "object", "properties": {"size": {"type": "string"}}}
                    }}}
                },
                {
                    "name": "v1", "served": true, "storage": true,
                    "additionalPrinterColumns": [
                        {"name": "Size", "type": "string", "jsonPath": ".spec.size"},
                        {"name": "Replicas", "type": "integer", "jsonPath": ".spec.replicas"},
                        {"name": "Owner", "type": "string", "jsonPath": ".spec.owner", "priority": 1}
                    ],
                    "schema": {"openAPIV3Schema": v1_schema()}
                }
            ]
        }
    })
}

/// The schema of `widgets.example.com` at `v1`.
pub(crate) fn v1_schema() -> Value {
    json!({
        "type": "object",
        "description": "Widget is a thing.",
        "required": ["spec"],
        "properties": {
            "apiVersion": {"type": "string", "description": "APIVersion defines the versioned schema."},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "spec": {
                "type": "object",
                "description": "WidgetSpec is what you want.\n\nSecond paragraph, not shown in the row.",
                "required": ["size", "containers"],
                "properties": {
                    "size": {
                        "type": "string",
                        "description": "How big the widget is.",
                        "enum": ["small", "medium", "large"],
                        "default": "small"
                    },
                    "replicas": {"type": "integer", "format": "int32", "default": 1},
                    "owner": {"type": "string"},
                    "timeout": {"x-kubernetes-int-or-string": true},
                    "expiresAt": {"type": "string", "format": "date-time"},
                    "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                    "ports": {"type": "array", "items": {"type": "integer"}},
                    "raw": {"type": "object", "x-kubernetes-preserve-unknown-fields": true},
                    "selector": {
                        "type": "object",
                        "properties": {
                            "matchLabels": {"type": "object", "additionalProperties": {"type": "string"}}
                        }
                    },
                    "containers": {
                        "type": "array",
                        "description": "Containers of the widget.",
                        "items": {
                            "type": "object",
                            "required": ["image", "name"],
                            "properties": {
                                "name": {"type": "string"},
                                "image": {"type": "string"},
                                "env": {
                                    "type": "array",
                                    "items": {"type": "object", "properties": {
                                        "name": {"type": "string"},
                                        "value": {"type": "string"}
                                    }}
                                }
                            }
                        }
                    }
                }
            },
            "status": {
                "type": "object",
                "properties": {"phase": {"type": "string", "enum": ["Pending", "Ready"]}}
            }
        }
    })
}

/// [`widget_crd_json`] as a `Resource`.
pub(crate) fn widget_crd() -> Resource {
    Resource::from_json(widget_crd_json()).expect("a resource")
}

/// A cluster-scoped CRD `fleets.example.com`, one version `v1`, without a schema.
pub(crate) fn fleet_crd_json() -> Value {
    json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": "fleets.example.com", "resourceVersion": "6"},
        "spec": {
            "group": "example.com",
            "scope": "Cluster",
            "names": {"plural": "fleets", "singular": "fleet", "kind": "Fleet"},
            "versions": [{"name": "v1", "served": true, "storage": true}]
        }
    })
}

// --- kinds as discovery serves them, and Table feed batches ------------------------------

use std::sync::Arc;

use oxikube_domain::ObjectMeta;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};

/// A kind as discovery serves it: listable, watchable, deletable.
pub(crate) fn kind(
    group: &str,
    version: &str,
    name: &str,
    plural: &str,
    namespaced: bool,
    preferred: bool,
) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced,
    }
}

/// The CRD kind (`apiextensions.k8s.io/v1`), cluster-scoped.
pub(crate) fn crd_kind() -> ResourceKind {
    kind(
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
        "customresourcedefinitions",
        false,
        true,
    )
}

/// `Widget` of `example.com` at `version`, namespaced.
pub(crate) fn widget_kind(version: &str, preferred: bool) -> ResourceKind {
    let mut kind = kind("example.com", version, "Widget", "widgets", true, preferred);
    kind.short_names = vec!["wd".into(), "wdg".into()];
    kind
}

/// `Fleet` of `example.com` at `v1`, cluster-scoped.
pub(crate) fn fleet_kind() -> ResourceKind {
    kind("example.com", "v1", "Fleet", "fleets", false, true)
}

/// `(name, type, priority)` as the server's column definitions.
pub(crate) fn columns(defs: &[(&str, &str, i32)]) -> Arc<[TableColumn]> {
    defs.iter()
        .map(|(name, ty, priority)| TableColumn {
            name: (*name).to_owned(),
            column_type: (*ty).to_owned(),
            priority: *priority,
            ..TableColumn::default()
        })
        .collect()
}

/// A Table batch of `source` with `columns` and one row per `(namespace, name, cells)`.
pub(crate) fn batch(
    source: TableSource,
    columns: Arc<[TableColumn]>,
    rows: &[(Option<&str>, &str, Vec<Value>)],
) -> TableBatch {
    let rows = rows
        .iter()
        .map(|(namespace, name, cells)| {
            let mut meta = ObjectMeta::named(*name);
            meta.namespace = namespace.map(Into::into);
            meta.resource_version = Some("1".into());
            TableRow {
                cells: cells.clone(),
                meta: Some(meta),
                object: None,
            }
        })
        .collect();
    TableBatch {
        columns: Some(columns),
        rows: DeltaBatch::from_deltas(vec![Delta::Restarted(rows)]),
        source,
    }
}
