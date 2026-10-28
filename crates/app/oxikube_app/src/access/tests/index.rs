//! Queries, binding scopes and subject expansion.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::access::Access;
use oxikube_domain::ids::Gvr;
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{BindingKind, GrantScope, RbacKind, RoleId, RoleKind, Subject};
use serde_json::json;

use super::{binding, index_of, role};
use crate::access::SubjectIndex;
use crate::store::{RowChange, RowOp, StoreObject};

fn pods() -> Gvr {
    Gvr::new("", "v1", "pods")
}

fn read_pods() -> serde_json::Value {
    json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]}])
}

/// `alice` reads pods in `shop` through a RoleBinding to a ClusterRole, `root` is cluster-admin
/// through a ClusterRoleBinding, `builder` (a ServiceAccount of `ci`) gets a Role through its
/// namespace's group.
fn cluster() -> SubjectIndex {
    index_of([
        role(None, "pod-reader", read_pods(), json!({})),
        role(
            None,
            "cluster-admin",
            json!([
                {"apiGroups": ["*"], "resources": ["*"], "verbs": ["*"]},
                {"nonResourceURLs": ["*"], "verbs": ["*"]},
            ]),
            json!({}),
        ),
        role(
            Some("ci"),
            "deployer",
            json!([
                {"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["patch"]},
                {"nonResourceURLs": ["/metrics"], "verbs": ["get"]},
            ]),
            json!({}),
        ),
        binding(
            Some("shop"),
            "alice-reads",
            ("ClusterRole", "pod-reader"),
            json!([{"kind": "User", "name": "alice"}]),
        ),
        binding(
            None,
            "root",
            ("ClusterRole", "cluster-admin"),
            json!([{"kind": "User", "name": "root"}]),
        ),
        binding(
            Some("ci"),
            "deployers",
            ("Role", "deployer"),
            json!([{"kind": "Group", "name": "system:serviceaccounts:ci"}]),
        ),
        binding(
            Some("ci"),
            "missing",
            ("Role", "not-there"),
            json!([{"kind": "ServiceAccount", "name": "builder"}]),
        ),
    ])
}

#[test]
fn a_role_binding_to_a_cluster_role_stays_namespaced() {
    let index = cluster();
    let alice = Subject::User("alice".into());
    assert_eq!(
        index.can(&alice, "list", &pods(), Some("shop"), None),
        Access::Granted
    );
    assert_eq!(
        index.can(&alice, "list", &pods(), Some("other"), None),
        Access::Denied
    );
    assert_eq!(
        index.can(&alice, "list", &pods(), None, None),
        Access::Denied,
        "all namespaces"
    );
    assert_eq!(
        index.can(&alice, "delete", &pods(), Some("shop"), None),
        Access::Denied
    );
    let scopes: Vec<GrantScope> = index
        .rules_for(&alice)
        .into_iter()
        .map(|r| r.scope)
        .collect();
    assert_eq!(scopes, [GrantScope::Namespace("shop".into())]);
}

#[test]
fn a_cluster_role_binding_grants_everywhere() {
    let index = cluster();
    let root = Subject::User("root".into());
    for namespace in [None, Some("shop"), Some("anything")] {
        assert_eq!(
            index.can(&root, "delete", &pods(), namespace, None),
            Access::Granted
        );
    }
    assert!(index.can_url(&root, "get", "/healthz"));
    assert!(!index.can_url(&Subject::User("alice".into()), "get", "/healthz"));
}

#[test]
fn a_service_account_gets_its_namespace_groups_but_no_urls_from_a_role_binding() {
    let index = cluster();
    let builder = Subject::service_account("ci", "builder");
    let deployments = Gvr::new("apps", "v1", "deployments");
    assert_eq!(
        index.can(&builder, "patch", &deployments, Some("ci"), None),
        Access::Granted
    );
    assert_eq!(
        index.can(&builder, "patch", &deployments, Some("shop"), None),
        Access::Denied
    );
    assert!(
        !index.can_url(&builder, "get", "/metrics"),
        "URLs are never namespaced"
    );
    let grants = index.grants_for(&builder);
    assert_eq!(grants.len(), 2);
    let via: Vec<String> = grants.iter().map(|g| g.via.to_string()).collect();
    assert_eq!(
        via,
        [
            "Group system:serviceaccounts:ci",
            "ServiceAccount ci/builder"
        ]
    );
    assert!(
        grants[1].rules.is_empty() && !grants[1].role_found,
        "missing role grants nothing"
    );
    assert!(grants[0].role_found);
    let another = Subject::service_account("ci", "other");
    assert_eq!(
        index.bindings_for(&another).len(),
        1,
        "the group binding only"
    );
}

#[test]
fn subjects_bindings_and_roles_are_listed() {
    let index = cluster();
    let subjects: Vec<String> = index.subjects().iter().map(ToString::to_string).collect();
    assert_eq!(
        subjects,
        [
            "User alice",
            "User root",
            "Group system:serviceaccounts:ci",
            "ServiceAccount ci/builder",
        ]
    );
    let alice = index.bindings_for(&Subject::User("alice".into()));
    assert_eq!(alice.len(), 1);
    assert_eq!(alice[0].kind, BindingKind::RoleBinding);
    assert_eq!(
        index.roles_granting("list", &pods()),
        [
            RoleId::cluster_role("cluster-admin"),
            RoleId::cluster_role("pod-reader")
        ]
    );
    assert_eq!(
        index.roles_granting("patch", &Gvr::new("apps", "v1", "deployments")),
        [
            RoleId::role("ci", "deployer"),
            RoleId::cluster_role("cluster-admin")
        ],
        "Roles sort before ClusterRoles"
    );
    assert_eq!(index.count(RbacKind::RoleBinding), 3);
    assert_eq!(index.count(RbacKind::ClusterRole), 2);
    assert!(index.role_rules(&RoleId::role("ci", "nope")).is_none());
    assert_eq!(RoleId::role("ci", "deployer").kind, RoleKind::Role);
}

#[test]
fn the_matrix_projects_the_subjects_rules_in_a_scope() {
    let index = cluster();
    let alice = Subject::User("alice".into());
    let matrix = index.matrix(&alice, &GrantScope::Namespace("shop".into()));
    assert_eq!(matrix.rows.len(), 1);
    assert_eq!(matrix.rows[0].resource, "pods");
    assert_eq!(matrix.rows[0].cell(Verb::List), Access::Granted);
    assert_eq!(matrix.rows[0].cell(Verb::Delete), Access::Denied);
    assert!(index.matrix(&alice, &GrantScope::Cluster).rows.is_empty());
}

#[test]
fn an_aggregated_cluster_role_is_resolved_from_its_selectors() {
    let index = index_of([
        role(None, "team", serde_json::Value::Null, json!({})),
        role(None, "team-pods", read_pods(), json!({"team": "true"})),
        binding(
            None,
            "team",
            ("ClusterRole", "team"),
            json!([{"kind": "Group", "name": "team"}]),
        ),
    ]);
    let member = Subject::Group("team".into());
    assert_eq!(
        index.can(&member, "get", &pods(), None, None),
        Access::Denied
    );
    // An update gives "team" an aggregation rule selecting "team-pods".
    let mut index = index;
    let aggregated = Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRole",
        "metadata": {"name": "team"},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"team": "true"}}]},
    }))
    .expect("a ClusterRole");
    let version = index.version();
    index.apply_rows(
        RbacKind::ClusterRole,
        &RowChange::Ops(vec![RowOp::Update {
            index: 0,
            object: Arc::new(StoreObject::Resource(aggregated)),
        }]),
    );
    assert!(index.version() > version);
    assert_eq!(
        index.can(&member, "get", &pods(), None, None),
        Access::Granted
    );
}

#[test]
fn grants_share_the_cached_role_rules_instead_of_copying_them() {
    let index = cluster();
    let root = Subject::User("root".into());
    let admin = RoleId {
        kind: RoleKind::ClusterRole,
        namespace: None,
        name: "cluster-admin".into(),
    };
    let cached = index.role_rules(&admin).expect("cluster-admin exists");
    let grants = index.grants_for(&root);
    assert!(
        Arc::ptr_eq(&grants[0].rules, &cached),
        "a cluster-wide grant hands out the cached rules"
    );

    // A RoleBinding's grant drops the URLs once, then shares that copy too.
    let builder = Subject::service_account("ci", "builder");
    let first = index.grants_for(&builder);
    let again = index.grants_for(&builder);
    assert!(Arc::ptr_eq(&first[0].rules, &again[0].rules));
    assert_eq!(first[0].rules.non_resource_rules().count(), 0);
}

#[test]
fn grants_in_keeps_the_grants_of_one_scope() {
    let index = cluster();
    let builder = Subject::service_account("ci", "builder");
    assert_eq!(index.grants_in(&builder, Some("ci")).len(), 2);
    assert!(index.grants_in(&builder, Some("shop")).is_empty());
    assert!(
        index.grants_in(&builder, None).is_empty(),
        "no cluster grant"
    );
    let root = Subject::User("root".into());
    assert_eq!(
        index.grants_in(&root, Some("shop")).len(),
        1,
        "a cluster-wide grant holds in every namespace"
    );
}
