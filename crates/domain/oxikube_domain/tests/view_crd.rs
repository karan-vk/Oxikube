//! `CrdSummary` (E12-S17): the CRD detail projection, printer column selection by priority, the
//! list's `Versions` and `Aliases` cells, and Kubernetes' version priority.

use std::cmp::Ordering;
use std::sync::Arc;

use oxikube_domain::ids::Scope;
use oxikube_domain::view::{
    ConditionStatus, CrdConversion, CrdSummary, crd_aliases_text, crd_versions_text, version_order,
};
use oxikube_domain::{Resource, ViewError};
use proptest::prelude::*;
use serde_json::{Value, json};

fn crd(spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": "widgets.example.com", "uid": "u", "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": spec,
        "status": status,
    }))
    .unwrap()
}

/// Three versions (alpha not served, beta deprecated, v1 storage with printer columns), a
/// conversion webhook and both conditions.
fn widgets() -> Resource {
    crd(
        json!({
            "group": "example.com",
            "scope": "Namespaced",
            "names": {
                "plural": "widgets", "singular": "widget", "kind": "Widget", "listKind": "WidgetList",
                "shortNames": ["wd", "widget"], "categories": ["all", "toys"]
            },
            "versions": [
                {"name": "v1alpha1", "served": false, "storage": false},
                {"name": "v1beta1", "served": true, "storage": false, "deprecated": true,
                 "deprecationWarning": "example.com/v1beta1 Widget is deprecated; use v1",
                 "schema": {"openAPIV3Schema": {"type": "object"}}},
                {"name": "v1", "served": true, "storage": true,
                 "subresources": {"status": {}, "scale": {"specReplicasPath": ".spec.replicas"}},
                 "schema": {"openAPIV3Schema": {"type": "object"}},
                 "additionalPrinterColumns": [
                    {"name": "Size", "type": "string", "jsonPath": ".spec.size"},
                    {"name": "Owner", "type": "string", "jsonPath": ".spec.owner", "priority": 1,
                     "description": "Who owns it"},
                    {"name": "Replicas", "type": "integer", "format": "int32", "jsonPath": ".spec.replicas"},
                    {"name": "Age", "type": "date", "jsonPath": ".metadata.creationTimestamp"}
                 ]}
            ],
            "conversion": {
                "strategy": "Webhook",
                "webhook": {
                    "conversionReviewVersions": ["v1"],
                    "clientConfig": {"service": {"namespace": "ops", "name": "conv", "port": 8443, "path": "/convert"}}
                }
            }
        }),
        json!({
            "conditions": [
                {"type": "NamesAccepted", "status": "True", "reason": "NoConflicts"},
                {"type": "Established", "status": "False", "reason": "Installing", "message": "not yet"}
            ],
            "storedVersions": ["v1beta1", "v1"]
        }),
    )
}

#[test]
fn the_detail_projection_reads_names_versions_conversion_and_conditions() {
    let s = CrdSummary::from_resource(&widgets()).unwrap();
    assert_eq!(&*s.name, "widgets.example.com");
    assert_eq!(&*s.group, "example.com");
    assert_eq!(s.scope, Scope::Namespaced);
    assert_eq!(&*s.names.kind, "Widget");
    assert_eq!(&*s.names.plural, "widgets");
    assert_eq!(s.names.singular.as_deref(), Some("widget"));
    assert_eq!(s.names.list_kind.as_deref(), Some("WidgetList"));
    assert_eq!(s.names.short_names.len(), 2);
    assert_eq!(s.names.categories, [Arc::from("all"), Arc::from("toys")]);

    let names: Vec<&str> = s.versions.iter().map(|v| &*v.name).collect();
    assert_eq!(names, ["v1alpha1", "v1beta1", "v1"]);
    let beta = s.version("v1beta1").unwrap();
    assert!(beta.served && beta.deprecated && !beta.storage);
    assert_eq!(
        beta.deprecation_warning.as_deref(),
        Some("example.com/v1beta1 Widget is deprecated; use v1")
    );
    let v1 = s.version("v1").unwrap();
    assert!(v1.storage && v1.has_schema && v1.status_subresource && v1.scale_subresource);
    assert!(!s.version("v1alpha1").unwrap().has_schema);
    assert_eq!(v1.printer_columns[2].format.as_deref(), Some("int32"));
    assert_eq!(&*v1.printer_columns[2].json_path, ".spec.replicas");
    assert_eq!(
        v1.printer_columns[1].description.as_deref(),
        Some("Who owns it")
    );

    assert_eq!(
        s.conversion,
        CrdConversion::Webhook {
            service: Some(Arc::from("ops/conv:8443/convert")),
            url: None,
            review_versions: vec![Arc::from("v1")],
        }
    );
    assert_eq!(s.conversion.strategy(), "Webhook");
    assert_eq!(s.names_accepted(), ConditionStatus::True);
    assert_eq!(s.established(), ConditionStatus::False);
    assert_eq!(s.conditions[1].message.as_deref(), Some("not yet"));
    assert_eq!(s.stored_versions, [Arc::from("v1beta1"), Arc::from("v1")]);
    assert!(s.created.is_some());
    assert_eq!(s.display_version().map(|v| &*v.name), Some("v1"));
}

#[test]
fn printer_columns_split_by_priority_as_kubectl_get_and_wide() {
    let s = CrdSummary::from_resource(&widgets()).unwrap();
    let v1 = s.version("v1").unwrap();
    let default: Vec<&str> = v1.default_columns().map(|c| &*c.name).collect();
    let wide: Vec<&str> = v1.wide_columns().map(|c| &*c.name).collect();
    assert_eq!(default, ["Size", "Replicas", "Age"], "priority 0, in order");
    assert_eq!(wide, ["Owner"], "priority > 0 only with -o wide");
    assert!(s.version("v1beta1").unwrap().printer_columns.is_empty());
}

#[test]
fn the_list_cells_match_the_servers_crd_table() {
    let res = widgets();
    // The API server's own CRD table prints `v1(storage)` (served versions, storage marked).
    assert_eq!(crd_versions_text(&res), "v1beta1,v1(storage)");
    let s = CrdSummary::from_resource(&res).unwrap();
    assert_eq!(s.versions_text(), crd_versions_text(&res));
    // Plural, singular, short names; `widget` appears once.
    assert_eq!(crd_aliases_text(&res), "widgets,widget,wd");
    let aliases = s.aliases();
    let aliases: Vec<&str> = aliases.iter().map(|a| &**a).collect();
    assert_eq!(aliases.join(","), crd_aliases_text(&res));
}

#[test]
fn a_bare_crd_degrades_to_defaults() {
    let s = CrdSummary::from_resource(&crd(json!({}), json!({}))).unwrap();
    assert_eq!(&*s.group, "");
    assert_eq!(s.scope, Scope::Namespaced);
    assert!(s.versions.is_empty());
    assert_eq!(s.conversion, CrdConversion::None);
    assert_eq!(s.established(), ConditionStatus::Unknown);
    assert!(s.display_version().is_none());
    assert_eq!(s.versions_text(), "");

    let cluster = crd(
        json!({"scope": "Cluster", "conversion": {"strategy": "None"},
               "versions": [{"name": "v2beta1", "served": true}, {"name": "v1", "served": true}]}),
        json!({}),
    );
    let s = CrdSummary::from_resource(&cluster).unwrap();
    assert_eq!(s.scope, Scope::Cluster);
    assert_eq!(s.conversion.strategy(), "None");
    // No storage version served: the newest served one (GA before beta).
    assert_eq!(s.display_version().map(|v| &*v.name), Some("v1"));
}

#[test]
fn another_kind_is_refused() {
    let pod =
        Resource::from_json(json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}))
            .unwrap();
    assert!(matches!(
        CrdSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
}

#[test]
fn versions_sort_ga_then_beta_then_alpha_newest_first() {
    let mut names = vec![
        "v1alpha1", "foo", "v1", "v2beta1", "v1beta2", "v2", "v1beta1",
    ];
    names.sort_by(|a, b| version_order(a, b));
    assert_eq!(
        names,
        [
            "v2", "v1", "v2beta1", "v1beta2", "v1beta1", "v1alpha1", "foo"
        ]
    );
    assert_eq!(version_order("v1", "v1"), Ordering::Equal);
}

proptest! {
    /// No shape of `spec` or `status` panics the projection.
    #[test]
    fn any_shape_reads_without_panicking(spec in arb_json(), status in arb_json()) {
        let res = crd(spec, status);
        let _ = CrdSummary::from_resource(&res).unwrap();
        let _ = crd_versions_text(&res);
        let _ = crd_aliases_text(&res);
    }
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-z]{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::from),
            prop::collection::btree_map(
                prop_oneof![
                    Just("versions".to_owned()),
                    Just("names".to_owned()),
                    Just("conversion".to_owned()),
                    Just("conditions".to_owned()),
                    Just("additionalPrinterColumns".to_owned()),
                    Just("name".to_owned()),
                    Just("served".to_owned()),
                    "[a-z]{1,5}",
                ],
                inner,
                0..4
            )
            .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}
