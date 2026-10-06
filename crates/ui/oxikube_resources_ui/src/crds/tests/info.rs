//! Reading a CRD: its names, scope and versions, and which version a table opens.

use oxikube_domain::ids::{Gvk, Scope};
use serde_json::json;

use super::fixture::{fleet_crd_json, widget_crd_json};
use crate::crds::{CrdInfo, crd_gvk, is_crd_kind, version_order};

#[test]
fn a_crd_reads_as_group_kind_scope_names_and_versions() {
    let info = CrdInfo::parse(&widget_crd_json()).expect("a CRD");
    assert_eq!(info.name, "widgets.example.com");
    assert_eq!(
        (info.group.as_str(), info.kind.as_str()),
        ("example.com", "Widget")
    );
    assert_eq!(info.plural, "widgets");
    assert_eq!(info.scope, Scope::Namespaced);
    assert_eq!(info.short_names, ["wd", "wdg"]);
    assert_eq!(info.categories, ["all"]);
    let names: Vec<_> = info.versions.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["v1alpha1", "v1beta1", "v1"]);
    let v1beta1 = &info.versions[1];
    assert!(v1beta1.served && !v1beta1.storage && v1beta1.deprecated && v1beta1.has_schema);
    assert_eq!(info.gvk("v1"), Gvk::new("example.com", "v1", "Widget"));

    let fleet = CrdInfo::parse(&fleet_crd_json()).expect("a CRD");
    assert_eq!(fleet.scope, Scope::Cluster);
    assert!(!fleet.versions[0].has_schema);
}

#[test]
fn something_that_is_not_a_crd_does_not_parse() {
    assert!(CrdInfo::parse(&json!({"metadata": {"name": "x"}})).is_none());
    assert!(CrdInfo::parse(&json!({"metadata": {"name": "x"}, "spec": {"group": "g"}})).is_none());
}

#[test]
fn a_table_opens_the_storage_version_when_it_is_served_else_the_newest_served() {
    let info = CrdInfo::parse(&widget_crd_json()).unwrap();
    assert_eq!(info.display_version().unwrap().name, "v1");

    // The storage version is no longer served: the newest served one is shown.
    let mut json = widget_crd_json();
    json["spec"]["versions"][2]["served"] = json!(false);
    json["spec"]["versions"][1]["served"] = json!(true);
    json["spec"]["versions"][0]["served"] = json!(true);
    let info = CrdInfo::parse(&json).unwrap();
    assert_eq!(info.display_version().unwrap().name, "v1beta1");

    // Nothing served: nothing to open.
    let mut none = widget_crd_json();
    for v in none["spec"]["versions"].as_array_mut().unwrap() {
        v["served"] = json!(false);
    }
    assert!(CrdInfo::parse(&none).unwrap().display_version().is_none());
}

#[test]
fn versions_sort_like_kubernetes_prioritises_them() {
    let mut names = vec![
        "v1alpha1", "v2beta1", "v1", "v10", "v2", "v1beta2", "v1beta1", "preview",
    ];
    names.sort_by(|a, b| version_order(a, b));
    assert_eq!(
        names,
        [
            "v10", "v2", "v1", "v2beta1", "v1beta2", "v1beta1", "v1alpha1", "preview"
        ]
    );
}

#[test]
fn the_crd_kind_is_recognised_in_any_version() {
    assert!(is_crd_kind(&crd_gvk()));
    assert!(is_crd_kind(&Gvk::new(
        "apiextensions.k8s.io",
        "v1beta1",
        "CustomResourceDefinition"
    )));
    assert!(!is_crd_kind(&Gvk::new(
        "example.com",
        "v1",
        "CustomResourceDefinition"
    )));
    assert!(!is_crd_kind(&Gvk::new("", "v1", "Pod")));
}
