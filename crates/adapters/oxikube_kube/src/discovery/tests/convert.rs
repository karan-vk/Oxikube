use k8s_openapi::apimachinery::pkg::apis::meta::v1::{APIGroup, APIResourceList};
use kube::core::discovery::v2::{APIGroupDiscovery, APIGroupDiscoveryList};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};
use serde_json::json;

use crate::discovery::convert::{
    Discovered, LegacyList, from_aggregated, from_legacy, preferred_version,
};

const AGGREGATED: &str = include_str!("../../../tests/fixtures/discovery/aggregated.json");

/// The fixture's core group followed by its other groups.
fn fixture() -> Vec<APIGroupDiscovery> {
    let doc: serde_json::Value = serde_json::from_str(AGGREGATED).expect("fixture");
    let mut groups = Vec::new();
    for key in ["api", "apis"] {
        let list: APIGroupDiscoveryList = serde_json::from_value(doc[key].clone()).expect("list");
        groups.extend(list.items);
    }
    groups
}

fn kinds() -> Vec<ResourceKind> {
    from_aggregated(&fixture())
        .into_iter()
        .map(|d| d.kind)
        .collect()
}

fn find<'a>(kinds: &'a [ResourceKind], group: &str, version: &str, kind: &str) -> &'a ResourceKind {
    let gvk = Gvk::new(group, version, kind);
    kinds
        .iter()
        .find(|k| k.gvk == gvk)
        .unwrap_or_else(|| panic!("{gvk} not discovered"))
}

#[test]
fn pods_carry_names_verbs_and_scope() {
    let kinds = kinds();
    let pod = find(&kinds, "", "v1", "Pod");
    assert_eq!(pod.plural, "pods");
    assert_eq!(pod.singular, "pod");
    assert_eq!(pod.short_names, ["po"]);
    assert_eq!(pod.categories, ["all"]);
    assert!(pod.namespaced);
    assert!(pod.preferred);
    assert_eq!(
        pod.verbs,
        VerbSet::from_names(Verb::ALL.iter().map(|v| v.as_str()))
    );
}

#[test]
fn core_group_is_empty_and_cluster_kinds_are_not_namespaced() {
    let kinds = kinds();
    let namespace = find(&kinds, "", "v1", "Namespace");
    assert!(namespace.gvk.is_core());
    assert!(!namespace.namespaced);
    assert_eq!(namespace.short_names, ["ns"]);
}

#[test]
fn subresources_are_not_listed() {
    let kinds = kinds();
    assert!(kinds.iter().all(|k| !k.plural.contains('/')));
    // `Scale` and `PodExecOptions` only exist as subresource response kinds in the fixture.
    assert!(
        kinds
            .iter()
            .all(|k| !matches!(&*k.gvk.kind, "Scale" | "PodExecOptions"))
    );
}

#[test]
fn kinds_without_get_or_list_are_dropped() {
    let kinds = kinds();
    assert!(
        kinds
            .iter()
            .all(|k| !matches!(&*k.gvk.kind, "Binding" | "TokenReview"))
    );
    let cs = find(&kinds, "", "v1", "ComponentStatus");
    assert_eq!(cs.verbs, VerbSet::from([Verb::Get, Verb::List]));
}

#[test]
fn first_listed_version_is_preferred_and_the_rest_are_kept() {
    let kinds = kinds();
    assert!(find(&kinds, "autoscaling", "v2", "HorizontalPodAutoscaler").preferred);
    assert!(!find(&kinds, "autoscaling", "v1", "HorizontalPodAutoscaler").preferred);
}

#[test]
fn custom_resources_are_converted_like_built_ins() {
    let kinds = kinds();
    let widget = find(&kinds, "test.oxikube.dev", "v1", "Widget");
    assert_eq!(widget.plural, "widgets");
    assert_eq!(widget.short_names, ["wd"]);
    assert_eq!(widget.categories, ["all"]);
    assert_eq!(widget.gvr().to_string(), "test.oxikube.dev/v1/widgets");
}

#[test]
fn api_resource_matches_what_kube_would_build() {
    let discovered: Vec<Discovered> = from_aggregated(&fixture());
    let find = |kind: &str| {
        discovered
            .iter()
            .find(|d| &*d.kind.gvk.kind == kind)
            .map(|d| d.api_resource.clone())
            .expect("kind")
    };
    let deployment = find("Deployment");
    assert_eq!(deployment.group, "apps");
    assert_eq!(deployment.version, "v1");
    assert_eq!(deployment.api_version, "apps/v1");
    assert_eq!(deployment.kind, "Deployment");
    assert_eq!(deployment.plural, "deployments");
    let pod = find("Pod");
    assert_eq!((pod.group.as_str(), pod.api_version.as_str()), ("", "v1"));
}

fn legacy_list(preferred: bool, doc: serde_json::Value) -> LegacyList {
    LegacyList {
        preferred,
        list: serde_json::from_value::<APIResourceList>(doc).expect("list"),
    }
}

#[test]
fn legacy_lists_convert_names_and_skip_subresources() {
    let list = legacy_list(
        true,
        json!({
            "kind": "APIResourceList",
            "groupVersion": "apps/v1",
            "resources": [
                {"name": "deployments", "singularName": "deployment", "namespaced": true, "kind": "Deployment",
                 "verbs": ["get", "list", "proxy"], "shortNames": ["deploy"], "categories": ["all"]},
                {"name": "deployments/scale", "singularName": "", "namespaced": true, "kind": "Scale", "verbs": ["get"]},
                {"name": "bindings", "singularName": "", "namespaced": true, "kind": "Binding", "verbs": ["create"]},
            ],
        }),
    );
    let kinds: Vec<_> = from_legacy(&[list]).into_iter().map(|d| d.kind).collect();
    assert_eq!(kinds.len(), 1);
    let deployment = &kinds[0];
    assert_eq!(deployment.gvk, Gvk::new("apps", "v1", "Deployment"));
    assert_eq!(deployment.short_names, ["deploy"]);
    assert_eq!(deployment.categories, ["all"]);
    assert_eq!(
        deployment.verbs,
        VerbSet::from([Verb::Get, Verb::List]),
        "`proxy` is not modelled"
    );
    assert!(deployment.preferred);
}

#[test]
fn legacy_core_list_has_the_empty_group() {
    let list = legacy_list(
        true,
        json!({
            "kind": "APIResourceList",
            "groupVersion": "v1",
            "resources": [{"name": "nodes", "singularName": "node", "namespaced": false, "kind": "Node", "verbs": ["get", "list"]}],
        }),
    );
    let discovered = from_legacy(&[list]);
    assert_eq!(discovered[0].kind.gvk, Gvk::new("", "v1", "Node"));
    assert_eq!(discovered[0].api_resource.api_version, "v1");
    assert!(discovered[0].kind.short_names.is_empty());
}

fn group(preferred: Option<&str>, versions: &[&str]) -> APIGroup {
    serde_json::from_value(json!({
        "name": "example.dev",
        "versions": versions.iter().map(|v| json!({"groupVersion": format!("example.dev/{v}"), "version": v})).collect::<Vec<_>>(),
        "preferredVersion": preferred.map(|v| json!({"groupVersion": format!("example.dev/{v}"), "version": v})),
    }))
    .expect("group")
}

#[test]
fn preferred_version_honours_the_server_then_falls_back_to_priority() {
    let versions = ["v1beta1", "v1", "v1alpha1", "v2beta1"];
    assert_eq!(
        preferred_version(&group(Some("v1beta1"), &versions)).as_deref(),
        Some("v1beta1")
    );
    assert_eq!(
        preferred_version(&group(None, &versions)).as_deref(),
        Some("v1")
    );
    assert_eq!(preferred_version(&group(None, &[])), None);
}
