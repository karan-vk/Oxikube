//! Reading Roles and bindings from JSON, subjects, scopes and aggregation.

use serde_json::json;

use super::super::{
    Binding, BindingKind, GrantScope, PolicyRule, RbacError, RbacKind, Role, RoleId, RoleKind,
    Subject, aggregated_from, aggregated_rules,
};
use crate::Resource;

fn resource(value: serde_json::Value) -> Resource {
    Resource::from_json(value).expect("test objects are valid resources")
}

fn cluster_role(name: &str, labels: serde_json::Value, rules: serde_json::Value) -> Role {
    Role::from_resource(&resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRole",
        "metadata": {"name": name, "labels": labels},
        "rules": rules,
    })))
    .expect("a ClusterRole")
}

#[test]
fn kinds_are_told_by_group_and_kind() {
    for kind in RbacKind::ALL {
        assert_eq!(RbacKind::of(&kind.gvk()), Some(kind));
    }
    let v1beta1 = crate::ids::Gvk::new("rbac.authorization.k8s.io", "v1beta1", "Role");
    assert_eq!(RbacKind::of(&v1beta1), Some(RbacKind::Role));
    assert_eq!(RbacKind::of(&crate::ids::Gvk::new("", "v1", "Role")), None);
    assert!(RbacKind::RoleBinding.is_namespaced() && !RbacKind::ClusterRole.is_namespaced());
}

#[test]
fn a_role_reads_its_rules_and_aggregation() {
    let role = Role::from_resource(&resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "Role",
        "metadata": {"name": "reader", "namespace": "demo"},
        "rules": [
            {"apiGroups": [""], "resources": ["pods"], "verbs": ["get"], "resourceNames": ["a"]},
            {"nonResourceURLs": ["/healthz"], "verbs": ["get"]},
            {"apiGroups": [""], "resources": ["pods", 7], "verbs": ["list"]},
        ],
    })))
    .expect("a Role");
    assert_eq!(role.id, RoleId::role("demo", "reader"));
    assert_eq!(role.rules.len(), 3);
    assert_eq!(
        role.rules[0],
        PolicyRule::resource(&["get"], &[""], &["pods"], &["a"])
    );
    assert_eq!(
        role.rules[1],
        PolicyRule::non_resource(&["get"], &["/healthz"])
    );
    assert_eq!(
        role.rules[2].resources,
        ["pods"],
        "non-string entries are skipped"
    );
    assert!(role.aggregation.is_none());
    assert_eq!(role.id.to_string(), "Role demo/reader");

    let aggregated = Role::from_resource(&resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRole",
        "metadata": {"name": "team"},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"team": "true"}}]},
    })))
    .expect("a ClusterRole");
    assert!(
        aggregated.rules.is_empty(),
        "no rules yet: the controller has not filled them"
    );
    assert_eq!(aggregated.aggregation.map(|a| a.selectors.len()), Some(1));
}

#[test]
fn wrong_or_broken_objects_are_errors() {
    let pod = resource(json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}));
    assert!(matches!(
        Role::from_resource(&pod),
        Err(RbacError::WrongKind { .. })
    ));
    assert!(matches!(
        Binding::from_resource(&pod),
        Err(RbacError::WrongKind { .. })
    ));
    let no_ref = resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRoleBinding",
        "metadata": {"name": "b"},
        "roleRef": {"kind": "Pod", "name": "x"},
    }));
    assert!(matches!(
        Binding::from_resource(&no_ref),
        Err(RbacError::InvalidRoleRef { .. })
    ));
}

#[test]
fn binding_scope_follows_the_binding_not_the_role() {
    let binding = |kind: &str, namespace: Option<&str>, role_kind: &str| {
        let mut metadata = json!({"name": "b"});
        if let Some(namespace) = namespace {
            metadata["namespace"] = json!(namespace);
        }
        Binding::from_resource(&resource(json!({
            "apiVersion": "rbac.authorization.k8s.io/v1",
            "kind": kind,
            "metadata": metadata,
            "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": role_kind, "name": "r"},
            "subjects": [
                {"kind": "User", "name": "alice"},
                {"kind": "ServiceAccount", "name": "builder"},
                {"kind": "Robot", "name": "r2"},
            ],
        })))
        .expect("a binding")
    };
    // A RoleBinding to a ClusterRole stays namespaced.
    let to_cluster_role = binding("RoleBinding", Some("demo"), "ClusterRole");
    assert_eq!(
        to_cluster_role.scope(),
        GrantScope::Namespace("demo".into())
    );
    assert_eq!(to_cluster_role.role_id(), Some(RoleId::cluster_role("r")));
    assert!(to_cluster_role.scope().covers(Some("demo")));
    assert!(!to_cluster_role.scope().covers(Some("other")));
    assert!(!to_cluster_role.scope().covers(None));
    // A RoleBinding to a Role looks the Role up in its own namespace.
    let to_role = binding("RoleBinding", Some("demo"), "Role");
    assert_eq!(to_role.role_id(), Some(RoleId::role("demo", "r")));
    // A ClusterRoleBinding is cluster-wide; one naming a Role grants nothing.
    let cluster = binding("ClusterRoleBinding", None, "ClusterRole");
    assert_eq!(cluster.scope(), GrantScope::Cluster);
    assert!(cluster.scope().covers(None) && cluster.scope().covers(Some("any")));
    assert_eq!(cluster.id.kind, BindingKind::ClusterRoleBinding);
    assert_eq!(binding("ClusterRoleBinding", None, "Role").role_id(), None);
    // Subjects: unknown kinds dropped, a namespace-less ServiceAccount takes the binding's.
    assert_eq!(
        to_role.subjects,
        [
            Subject::User("alice".into()),
            Subject::service_account("demo", "builder")
        ]
    );
    assert_eq!(cluster.subjects, [Subject::User("alice".into())]);
}

#[test]
fn service_accounts_expand_to_their_user_and_groups() {
    let sa = Subject::service_account("demo", "builder");
    let info = sa.user_info();
    assert_eq!(
        info.user.as_deref(),
        Some("system:serviceaccount:demo:builder")
    );
    let groups: Vec<&str> = info.groups.iter().map(|g| &**g).collect();
    assert_eq!(
        groups,
        [
            "system:authenticated",
            "system:serviceaccounts",
            "system:serviceaccounts:demo"
        ]
    );
    assert!(info.is(&sa));
    assert!(info.is(&Subject::User("system:serviceaccount:demo:builder".into())));
    assert!(info.is(&Subject::Group("system:serviceaccounts:demo".into())));
    assert!(!info.is(&Subject::Group("system:serviceaccounts:other".into())));
    assert_eq!(
        Subject::from_username("system:serviceaccount:demo:builder"),
        sa,
        "a user name maps back"
    );
    assert_eq!(
        Subject::from_username("alice"),
        Subject::User("alice".into())
    );
    for not_a_service_account in ["system:serviceaccount:demo", "system:serviceaccount:a:b:c"] {
        assert!(matches!(
            Subject::from_username(not_a_service_account),
            Subject::User(_)
        ));
    }
    assert_eq!(
        info.subjects().len(),
        5,
        "user twice (both spellings) and three groups"
    );
    assert_eq!(sa.to_string(), "ServiceAccount demo/builder");
}

#[test]
fn users_and_groups_are_authenticated_unless_anonymous() {
    let alice = Subject::User("alice".into()).user_info();
    assert!(alice.is(&Subject::Group("system:authenticated".into())));
    assert!(!alice.is(&Subject::Group("system:unauthenticated".into())));
    let team = Subject::Group("team".into()).user_info();
    assert_eq!(team.user, None);
    assert!(team.is(&Subject::Group("team".into())));
    assert!(
        !team.is(&Subject::User("alice".into())),
        "a group has no user of its own"
    );
    let anonymous = Subject::User("system:anonymous".into()).user_info();
    assert!(anonymous.is(&Subject::Group("system:unauthenticated".into())));
    assert!(!anonymous.is(&Subject::Group("system:authenticated".into())));
}

#[test]
fn aggregation_resolves_selectors_recursively_and_unions_live_rules() {
    let pods = json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}]);
    let logs = json!([{"apiGroups": [""], "resources": ["pods/log"], "verbs": ["get"]}]);
    let leaf = cluster_role("leaf", json!({"to-middle": "true"}), logs);
    let middle = Role::from_resource(&resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRole",
        "metadata": {"name": "middle", "labels": {"to-top": "true"}},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"to-middle": "true"}}]},
    })))
    .expect("a ClusterRole");
    let top = Role::from_resource(&resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRole",
        "metadata": {"name": "top", "labels": {"to-middle": "true"}},
        "aggregationRule": {"clusterRoleSelectors": [
            {"matchExpressions": [{"key": "to-top", "operator": "Exists"}]},
        ]},
        "rules": pods,
    })))
    .expect("a ClusterRole");
    let all = [&leaf, &middle, &top];
    let rules = aggregated_rules(&top, &all);
    let expected = Role {
        rules: vec![
            PolicyRule::resource(&["get"], &[""], &["pods"], &[]),
            PolicyRule::resource(&["get"], &[""], &["pods/log"], &[]),
        ],
        ..leaf.clone()
    };
    assert_eq!(
        rules,
        expected.normalised(),
        "live rules plus both levels, cycle-safe"
    );
    assert_eq!(
        aggregated_from(&top, &all),
        [RoleId::cluster_role("middle")]
    );
    assert_eq!(
        aggregated_from(&middle, &all),
        [RoleId::cluster_role("leaf"), RoleId::cluster_role("top")]
    );
    assert!(aggregated_from(&leaf, &all).is_empty());
    assert_eq!(leaf.id.kind, RoleKind::ClusterRole);
}
