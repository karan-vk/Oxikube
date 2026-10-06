//! The sidebar's access review and custom-resource discovery over testkit fakes.

use futures::executor::block_on;
use oxikube_domain::access::{AccessRequirement, AccessRule, AccessRules};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_testkit::{AccessCall, FakeAccessReviewPort, FakeDiscoveryPort};

use super::{AccessOutcome, discover_custom_resources, review_access};

fn list(group: &str, resource: &str) -> [AccessRequirement; 1] {
    [AccessRequirement::list(group, resource)]
}

fn grant_list(group: &str, resource: &str) -> AccessRules {
    AccessRules::none().with_rule(AccessRule::granting(&["list"], &[group], &[resource], &[]))
}

#[test]
fn a_restricted_user_is_offered_only_what_they_may_list() {
    let access = FakeAccessReviewPort::new().with_rules(grant_list("", "pods"));
    let outcome = block_on(review_access(&access, &NamespaceSelection::single("dev")));
    assert!(outcome.offers(&list("", "pods")));
    assert!(!outcome.offers(&list("", "secrets")));
    assert!(!outcome.offers(&list("", "nodes")));
    assert_eq!(outcome.warning(), None);
    // A section covering several kinds shows when one of them is listable.
    assert!(outcome.offers(&[
        AccessRequirement::list("", "secrets"),
        AccessRequirement::list("", "pods"),
    ]));
    assert!(outcome.offers(&[]), "an entry that needs nothing is shown");
}

#[test]
fn a_full_admin_sees_everything() {
    let access = FakeAccessReviewPort::new();
    let outcome = block_on(review_access(&access, &NamespaceSelection::All));
    for (group, resource) in [
        ("", "nodes"),
        ("apps", "deployments"),
        ("rbac.authorization.k8s.io", "roles"),
        ("argoproj.io", "applications"),
    ] {
        assert!(outcome.offers(&list(group, resource)), "{group}/{resource}");
    }
}

#[test]
fn a_failed_review_fails_open_with_a_warning() {
    let access = FakeAccessReviewPort::new();
    access
        .script()
        .rules
        .push_err(OxiError::new(ErrorKind::Network, "connection reset"));
    let outcome = block_on(review_access(&access, &NamespaceSelection::All));
    assert!(matches!(outcome, AccessOutcome::Failed { .. }));
    assert!(
        outcome.offers(&list("", "secrets")),
        "hiding by mistake is worse"
    );
    assert_eq!(outcome.warning(), Some("connection reset"));
}

#[test]
fn a_partial_review_never_hides_what_it_did_not_list() {
    let access = FakeAccessReviewPort::new().with_rules(grant_list("", "pods").into_partial());
    let outcome = block_on(review_access(&access, &NamespaceSelection::All));
    assert!(outcome.offers(&list("", "secrets")));
}

#[test]
fn all_namespaces_asks_once_and_a_set_asks_only_for_its_namespaces() {
    let access = FakeAccessReviewPort::new();
    block_on(review_access(&access, &NamespaceSelection::All));
    assert_eq!(access.recorded_calls(), [AccessCall::Rules(None)]);

    access.clear_calls();
    block_on(review_access(
        &access,
        &NamespaceSelection::from_names(["b", "a"]),
    ));
    assert_eq!(
        access.recorded_calls(),
        [
            AccessCall::Rules(Some("a".into())),
            AccessCall::Rules(Some("b".into())),
        ],
        "no cluster-wide ask: it would read the probe namespace's Roles"
    );
}

#[test]
fn another_namespaces_grants_do_not_leak_into_the_selection() {
    // What a cluster-wide ask returns is the probe namespace's (`default`) rules: a Role there
    // that lists pods. The user selected `team-b`, where they may list nothing.
    let access = FakeAccessReviewPort::new()
        .with_rules(grant_list("", "pods"))
        .with_namespace_rules("team-b", AccessRules::none());
    let outcome = block_on(review_access(
        &access,
        &NamespaceSelection::single("team-b"),
    ));
    assert!(
        !outcome.offers(&list("", "pods")),
        "default's Role is not team-b's"
    );
}

#[test]
fn all_namespaces_never_hides_namespaced_resources_the_probe_namespace_did_not_grant() {
    // A Role bound only in `dev`: the cluster-wide review (in `default`) lists nothing.
    let access = FakeAccessReviewPort::new().with_rules(AccessRules::none());
    let outcome = block_on(review_access(&access, &NamespaceSelection::All));
    assert!(matches!(&outcome, AccessOutcome::Reviewed(rules) if rules.namespaces_unseen));
    assert!(
        outcome.offers(&list("", "pods")),
        "unknown is shown, not hidden"
    );
    assert!(outcome.offers(&list("apps", "deployments")));
    // Cluster-scoped kinds only come from ClusterRoleBindings, which the review sees in full.
    for (group, resource) in [
        ("", "nodes"),
        ("", "namespaces"),
        ("", "persistentvolumes"),
        ("storage.k8s.io", "storageclasses"),
        ("rbac.authorization.k8s.io", "clusterroles"),
    ] {
        assert!(!outcome.offers(&list(group, resource)), "{resource}");
    }
    let nodes = FakeAccessReviewPort::new().with_rules(grant_list("", "nodes"));
    let nodes = block_on(review_access(&nodes, &NamespaceSelection::All));
    assert!(
        nodes.offers(&list("", "nodes")),
        "a granted cluster-scoped kind shows"
    );
    // Naming a namespace gives the precise answer.
    let dev = block_on(review_access(&access, &NamespaceSelection::single("dev")));
    assert!(!dev.offers(&list("", "pods")));
}

#[test]
fn the_selected_namespaces_rules_are_unioned() {
    let access = FakeAccessReviewPort::new()
        .with_rules(AccessRules::none())
        .with_namespace_rules("dev", grant_list("apps", "deployments"))
        .with_namespace_rules("ops", grant_list("", "secrets"));
    let both = block_on(review_access(
        &access,
        &NamespaceSelection::from_names(["dev", "ops"]),
    ));
    assert!(both.offers(&list("apps", "deployments")));
    assert!(both.offers(&list("", "secrets")));
    assert!(!both.offers(&list("", "nodes")));

    let dev_only = block_on(review_access(&access, &NamespaceSelection::single("dev")));
    assert!(dev_only.offers(&list("apps", "deployments")));
    assert!(
        !dev_only.offers(&list("", "secrets")),
        "ops is not selected"
    );
}

#[test]
fn one_failing_namespace_review_fails_open() {
    let access = FakeAccessReviewPort::new().with_rules(AccessRules::none());
    access.script().rules.push_ok(AccessRules::none());
    access
        .script()
        .rules
        .push_err(OxiError::forbidden("rules review denied"));
    let outcome = block_on(review_access(
        &access,
        &NamespaceSelection::from_names(["dev", "ops"]),
    ));
    assert!(outcome.warning().is_some());
    assert!(outcome.offers(&list("", "pods")));
}

fn kind(group: &str, kind: &str, plural: &str, verbs: &[Verb], preferred: bool) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", kind),
        preferred,
        plural: plural.into(),
        singular: String::new(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: verbs.iter().copied().collect::<VerbSet>(),
        namespaced: true,
    }
}

#[test]
fn custom_resources_are_the_non_builtin_listable_kinds_grouped_by_group() {
    let crud = [Verb::Get, Verb::List, Verb::Watch];
    let discovery = FakeDiscoveryPort::new().with_kinds([
        kind("", "Pod", "pods", &crud, true),
        kind("apps", "Deployment", "deployments", &crud, true),
        kind("argoproj.io", "Application", "applications", &crud, true),
        kind("argoproj.io", "AppProject", "appprojects", &crud, true),
        kind(
            "cert-manager.io",
            "Certificate",
            "certificates",
            &crud,
            true,
        ),
        // Not listable, and an older served version: both left out.
        kind(
            "cert-manager.io",
            "Challenge",
            "challenges",
            &[Verb::Get],
            true,
        ),
        kind(
            "cert-manager.io",
            "Certificate",
            "certificates",
            &crud,
            false,
        ),
    ]);
    let groups = block_on(discover_custom_resources(&discovery)).unwrap();
    let shape: Vec<(&str, Vec<&str>)> = groups
        .iter()
        .map(|g| {
            (
                g.group.as_str(),
                g.kinds.iter().map(|k| k.kind.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("argoproj.io", vec!["AppProject", "Application"]),
            ("cert-manager.io", vec!["Certificate"]),
        ]
    );
}

#[test]
fn a_group_shows_only_the_kinds_the_user_may_list_and_hides_when_none() {
    let crud = [Verb::Get, Verb::List];
    let discovery = FakeDiscoveryPort::new().with_kinds([
        kind("argoproj.io", "Application", "applications", &crud, true),
        kind("argoproj.io", "AppProject", "appprojects", &crud, true),
        kind(
            "cert-manager.io",
            "Certificate",
            "certificates",
            &crud,
            true,
        ),
    ]);
    let groups = block_on(discover_custom_resources(&discovery)).unwrap();
    let rules = grant_list("argoproj.io", "applications");
    let outcome = AccessOutcome::Reviewed(rules);
    let visible: Vec<_> = groups
        .iter()
        .filter_map(|g| g.visible(|reqs| outcome.offers(reqs)))
        .collect();
    assert_eq!(visible.len(), 1, "cert-manager.io has nothing listable");
    assert_eq!(visible[0].group, "argoproj.io");
    assert_eq!(visible[0].kinds.len(), 1);
    assert_eq!(visible[0].kinds[0].kind, "Application");

    let none = AccessOutcome::Reviewed(AccessRules::none());
    assert!(
        groups
            .iter()
            .all(|g| g.visible(|r| none.offers(r)).is_none())
    );
    let open = AccessOutcome::Failed { reason: "x".into() };
    assert_eq!(
        groups
            .iter()
            .filter_map(|g| g.visible(|r| open.offers(r)))
            .count(),
        2
    );
}

#[test]
fn a_discovery_failure_is_the_ports_error() {
    let discovery = FakeDiscoveryPort::new();
    discovery
        .script()
        .discover
        .push_err(OxiError::new(ErrorKind::Timeout, "slow"));
    let err = block_on(discover_custom_resources(&discovery)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
}

#[test]
fn a_custom_kind_carries_its_preferred_version_and_scope_as_a_count_target() {
    let crud = [Verb::Get, Verb::List, Verb::Watch];
    let mut cluster_scoped = kind("example.com", "Fleet", "fleets", &crud, true);
    cluster_scoped.namespaced = false;
    let mut old = kind("example.com", "Widget", "widgets", &crud, false);
    old.gvk = Gvk::new("example.com", "v1alpha1", "Widget");
    let mut new = kind("example.com", "Widget", "widgets", &crud, true);
    new.gvk = Gvk::new("example.com", "v1beta1", "Widget");
    let discovery = FakeDiscoveryPort::new().with_kinds([old, new, cluster_scoped]);

    let groups = block_on(discover_custom_resources(&discovery)).unwrap();
    let group = &groups[0];
    let widget = group.kinds.iter().find(|k| k.kind == "Widget").unwrap();
    assert_eq!(widget.version, "v1beta1", "the preferred version");
    let target = widget.count_target(&group.group);
    assert_eq!(target.gvk, Gvk::new("example.com", "v1beta1", "Widget"));
    assert_eq!(target.scope, oxikube_domain::ids::Scope::Namespaced);
    let fleet = group.kinds.iter().find(|k| k.kind == "Fleet").unwrap();
    assert_eq!(
        fleet.count_target(&group.group).scope,
        oxikube_domain::ids::Scope::Cluster
    );
}
