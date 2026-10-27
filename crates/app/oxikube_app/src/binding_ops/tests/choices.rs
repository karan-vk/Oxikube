//! [`BindingChoices`] and [`role_of`] over the fakes.

use futures::FutureExt as _;
use oxikube_domain::rbac::{Binding, RoleKind};
use oxikube_domain::{OxiError, Resource};
use serde_json::json;

use super::role_binding;
use crate::binding_ops::{BindingChoices, role_of};
use crate::testing::{Harness, id};

fn object(value: serde_json::Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn seeded() -> Harness {
    let h = Harness::with_extra(|_, _, _| Ok(()));
    h.connect("a", false);
    let port = h.resources("a");
    for value in [
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
               "metadata": {"name": "reader", "namespace": "demo"}, "rules": []}),
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
               "metadata": {"name": "other", "namespace": "elsewhere"}, "rules": []}),
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
               "metadata": {"name": "view"}, "rules": []}),
        json!({"apiVersion": "v1", "kind": "ServiceAccount",
               "metadata": {"name": "builder", "namespace": "demo"}}),
        role_binding("view", None),
    ] {
        port.insert(object(value));
    }
    port.insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
        "metadata": {"name": "crb"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
        "subjects": [{"kind": "Group", "name": "team"}, {"kind": "User", "name": "bob"}],
    })));
    h
}

#[test]
fn the_pickers_offer_what_the_cluster_has_and_the_names_bindings_already_use() {
    let h = seeded();
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("fakes answer at once")
        .expect("read");
    assert_eq!(choices.cluster_roles.len(), 1);
    assert_eq!(choices.roles.len(), 2);
    assert_eq!(choices.service_accounts.len(), 1);
    let users: Vec<&str> = choices.users.iter().map(|u| &**u).collect();
    assert_eq!(users, ["ana", "bob"]);
    let groups: Vec<&str> = choices.groups.iter().map(|u| &**u).collect();
    assert_eq!(groups, ["team"]);
    assert!(choices.notes.is_empty(), "{:?}", choices.notes);
}

#[test]
fn a_cluster_role_binding_is_offered_only_cluster_roles_and_a_role_binding_its_own_namespace() {
    let h = seeded();
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("at once")
        .expect("read");
    let names = |offered: Vec<(RoleKind, std::sync::Arc<str>)>| -> Vec<String> {
        offered
            .iter()
            .map(|(kind, name)| format!("{}/{name}", kind.as_str()))
            .collect()
    };
    assert_eq!(names(choices.offered_roles(None)), ["ClusterRole/view"]);
    assert_eq!(
        names(choices.offered_roles(Some("demo"))),
        ["Role/reader", "ClusterRole/view"]
    );
    assert_eq!(
        names(choices.offered_roles(Some("empty"))),
        ["ClusterRole/view"]
    );
}

#[test]
fn a_list_that_is_refused_is_noted_and_the_rest_still_loads() {
    let h = seeded();
    h.resources("a")
        .script()
        .list
        .push_err(OxiError::forbidden("serviceaccounts is forbidden"));
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("at once")
        .expect("read");
    assert_eq!(choices.notes.len(), 1, "{:?}", choices.notes);
    assert!(choices.notes[0].contains("type the name instead"));
}

#[test]
fn the_role_a_binding_names_is_read_and_a_missing_one_is_none() {
    let h = seeded();
    let binding = Binding::from_resource(&object(role_binding("view", None))).expect("binding");
    let role = role_of(&h.manager, &id("a"), &binding)
        .now_or_never()
        .expect("at once")
        .expect("read");
    assert_eq!(role.map(|r| r.id.name.to_string()), Some("view".to_owned()));
    let gone = Binding::from_resource(&object(role_binding("gone", None))).expect("binding");
    assert!(
        role_of(&h.manager, &id("a"), &gone)
            .now_or_never()
            .expect("at once")
            .expect("read")
            .is_none()
    );
}
