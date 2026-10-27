//! The facts read from the object and the roles read from the index.

use oxikube_app::access::PolicyStatus;

use oxikube_domain::rbac::{GrantScope, RbacKind, Subject};
use serde_json::json;

use super::{binding, cluster, cluster_role, index_of, service_account};
use crate::service_account::{Automount, BoundRoles, ServiceAccountFacts};

#[test]
fn an_account_with_no_secrets_reads_as_empty_and_unset() {
    let facts = ServiceAccountFacts::from_resource(&service_account(json!({})));
    assert!(facts.secrets.is_empty());
    assert!(facts.image_pull_secrets.is_empty());
    assert_eq!(facts.automount, Automount::Unset);
    assert_eq!(&*facts.namespace, "shop");
    assert_eq!(facts.subject(), Subject::service_account("shop", "ci"));
}

#[test]
fn one_and_many_secrets_keep_their_order_and_skip_nameless_entries() {
    let one = ServiceAccountFacts::from_resource(&service_account(json!({
        "secrets": [{"name": "ci-token"}],
    })));
    assert_eq!(
        one.secrets.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["ci-token"]
    );

    let many = ServiceAccountFacts::from_resource(&service_account(json!({
        "secrets": [{"name": "a"}, {}, {"name": "b"}, {"name": "c"}],
        "imagePullSecrets": [{"name": "registry"}, {"name": "mirror"}],
    })));
    assert_eq!(
        many.secrets.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(
        many.image_pull_secrets
            .iter()
            .map(|s| &**s)
            .collect::<Vec<_>>(),
        ["registry", "mirror"]
    );
}

#[test]
fn automount_distinguishes_unset_from_false_and_explains_the_default() {
    let read = |value| {
        ServiceAccountFacts::from_resource(&service_account(json!({
            "automountServiceAccountToken": value
        })))
        .automount
    };
    assert_eq!(read(json!(true)), Automount::Enabled);
    assert_eq!(read(json!(false)), Automount::Disabled);
    assert_eq!(Automount::Unset.label(), "not set");
    assert!(Automount::Unset.explanation().contains("default is true"));
}

fn ci() -> Subject {
    Subject::service_account("shop", "ci")
}

fn index() -> oxikube_app::access::SubjectIndex {
    index_of(vec![
        cluster_role("viewer"),
        binding(
            Some("shop"),
            "ci-views",
            "viewer",
            json!([{"kind": "ServiceAccount", "name": "ci", "namespace": "shop"}]),
        ),
        binding(
            None,
            "everyone-views",
            "viewer",
            json!([{"kind": "Group", "name": "system:serviceaccounts", "apiGroup": "rbac.authorization.k8s.io"}]),
        ),
        binding(
            Some("shop"),
            "someone-else",
            "viewer",
            json!([{"kind": "ServiceAccount", "name": "other", "namespace": "shop"}]),
        ),
    ])
}

#[test]
fn bound_roles_show_scope_binding_and_the_group_they_came_through() {
    let BoundRoles::Known { roles, missing } =
        BoundRoles::of_index(&index(), PolicyStatus::Ready, &ci())
    else {
        panic!("the feeds are ready");
    };
    assert!(missing.is_empty());
    assert_eq!(roles.len(), 2, "{roles:?}");
    let namespaced = roles
        .iter()
        .find(|role| matches!(role.scope, GrantScope::Namespace(_)))
        .expect("the RoleBinding");
    assert_eq!(namespaced.scope_label(), "namespace shop");
    assert_eq!(&*namespaced.binding.name, "ci-views");
    assert!(namespaced.via_group.is_none());
    assert!(namespaced.role_found);
    let cluster_wide = roles
        .iter()
        .find(|role| role.scope == GrantScope::Cluster)
        .expect("the ClusterRoleBinding");
    assert_eq!(cluster_wide.scope_label(), "cluster-wide");
    assert_eq!(
        cluster_wide.via_group.as_deref(),
        Some("system:serviceaccounts")
    );
}

#[test]
fn links_point_at_the_role_and_the_binding() {
    let BoundRoles::Known { roles, .. } =
        BoundRoles::of_index(&index(), PolicyStatus::Ready, &ci())
    else {
        panic!("ready");
    };
    let cluster = cluster();
    let namespaced = roles
        .iter()
        .find(|role| &*role.binding.name == "ci-views")
        .expect("ci-views");
    let role = namespaced.role_target(&cluster);
    assert_eq!(&*role.gvk.kind, "ClusterRole");
    assert_eq!(role.namespace(), None, "a ClusterRole is cluster-scoped");
    let binding = namespaced.binding_target(&cluster);
    assert_eq!(&*binding.gvk.kind, "RoleBinding");
    assert_eq!(binding.namespace(), Some("shop"));
    assert_eq!(&*binding.name, "ci-views");
}

#[test]
fn loading_is_not_empty_and_a_refused_kind_is_reported() {
    let index = index();
    assert_eq!(
        BoundRoles::of_index(&index, PolicyStatus::Warming, &ci()),
        BoundRoles::Loading,
        "feeds still being listed must not read as no roles"
    );
    // The group binding of the fixture reaches every account, so "none" needs an empty index.
    let none = BoundRoles::of_index(&index_of(Vec::new()), PolicyStatus::Ready, &ci());
    assert!(none.is_empty());
    assert!(matches!(none, BoundRoles::Known { .. }), "known and empty");

    let partial = PolicyStatus::Partial(vec![(RbacKind::ClusterRole, "forbidden".to_owned())]);
    let BoundRoles::Known { roles, missing } = BoundRoles::of_index(&index, partial, &ci()) else {
        panic!("a partial answer is still an answer");
    };
    assert_eq!(roles.len(), 2);
    assert_eq!(missing, [(RbacKind::ClusterRole, "forbidden".to_owned())]);
}
