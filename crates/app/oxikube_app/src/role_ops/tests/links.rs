//! [`RoleLinks`](crate::role_ops::RoleLinks) and [`binds`](crate::role_ops::binds) over fakes.

use futures::FutureExt as _;
use oxikube_domain::Resource;
use oxikube_domain::rbac::{Binding, Role, RoleId};
use serde_json::json;

use super::cluster_role;
use crate::role_ops::{RoleLinks, binds};
use crate::testing::{Harness, id};

fn object(value: serde_json::Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn binding(
    kind: &str,
    namespace: Option<&str>,
    name: &str,
    role_kind: &str,
    role: &str,
) -> Resource {
    let mut metadata = json!({"name": name});
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }
    object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": kind, "metadata": metadata,
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": role_kind, "name": role},
        "subjects": [{"kind": "User", "name": "alice"}],
    }))
}

#[test]
fn a_role_is_named_by_its_own_namespace_and_a_cluster_role_by_any_binding() {
    let in_demo = Binding::from_resource(&binding("RoleBinding", Some("demo"), "b", "Role", "r"))
        .expect("binding");
    let in_other = Binding::from_resource(&binding("RoleBinding", Some("other"), "b", "Role", "r"))
        .expect("binding");
    let cluster_wide = Binding::from_resource(&binding(
        "ClusterRoleBinding",
        None,
        "b",
        "ClusterRole",
        "r",
    ))
    .expect("binding");
    assert!(binds(&in_demo, &RoleId::role("demo", "r")));
    assert!(!binds(&in_other, &RoleId::role("demo", "r")));
    assert!(binds(&cluster_wide, &RoleId::cluster_role("r")));
    assert!(
        !binds(&cluster_wide, &RoleId::role("demo", "r")),
        "kinds differ"
    );
}

#[test]
fn bindings_of_a_role_are_counted_across_namespaces_and_kinds() {
    let h = Harness::with_extra(|_, _, _| Ok(()));
    h.connect("a", false);
    let port = h.resources("a");
    for resource in [
        binding("ClusterRoleBinding", None, "crb", "ClusterRole", "view"),
        binding("RoleBinding", Some("one"), "rb1", "ClusterRole", "view"),
        binding("RoleBinding", Some("two"), "rb2", "ClusterRole", "view"),
        binding("RoleBinding", Some("two"), "rb3", "ClusterRole", "edit"),
    ] {
        port.insert(resource);
    }
    let links = RoleLinks::new(h.manager);
    let count = links
        .binding_count(&id("a"), &RoleId::cluster_role("view"))
        .now_or_never()
        .expect("fakes answer at once")
        .expect("count");
    assert_eq!(count, 3);
}

#[test]
fn an_aggregated_cluster_role_lists_the_roles_its_selector_matches() {
    let h = Harness::with_extra(|_, _, _| Ok(()));
    h.connect("a", false);
    let port = h.resources("a");
    let labelled = |name: &str, label: bool| {
        let mut role = cluster_role(None);
        role["metadata"] =
            json!({"name": name, "labels": if label { json!({"agg": "yes"}) } else { json!({}) }});
        object(role)
    };
    port.insert(labelled("b-part", true));
    port.insert(labelled("a-part", true));
    port.insert(labelled("other", false));
    let aggregate = object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "all"},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"agg": "yes"}}]},
        "rules": [],
    }));
    port.insert(aggregate.clone());
    let role = Role::from_resource(&aggregate).expect("role");
    let links = RoleLinks::new(h.manager);
    let from = links
        .contributors(&id("a"), &role)
        .now_or_never()
        .expect("fakes answer at once")
        .expect("contributors");
    let names: Vec<&str> = from.iter().map(|id| &*id.name).collect();
    assert_eq!(names, ["a-part", "b-part"]);
    let plain = Role::from_resource(&object(cluster_role(None))).expect("role");
    assert!(
        links
            .contributors(&id("a"), &plain)
            .now_or_never()
            .expect("at once")
            .expect("empty")
            .is_empty()
    );
}
