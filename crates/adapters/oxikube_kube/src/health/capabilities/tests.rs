use oxikube_domain::{Capabilities as C, Capability};

use super::*;

fn rule(groups: &[&str], resources: &[&str], verbs: &[&str]) -> AccessRule {
    let own = |s: &[&str]| s.iter().map(|s| (*s).to_owned()).collect();
    AccessRule {
        api_groups: own(groups),
        resources: own(resources),
        verbs: own(verbs),
        resource_names: vec![],
    }
}

fn named(mut r: AccessRule, names: &[&str]) -> AccessRule {
    r.resource_names = names.iter().map(|s| (*s).to_owned()).collect();
    r
}

fn complete(rules: Vec<AccessRule>) -> RulesSnapshot {
    RulesSnapshot {
        rules,
        ..Default::default()
    }
}

fn partial(rules: Vec<AccessRule>) -> RulesSnapshot {
    RulesSnapshot {
        rules,
        incomplete: true,
        evaluation_error: None,
    }
}

#[test]
fn cluster_admin_wildcards_grant_everything_rbac_decides() {
    let r = capabilities_from_rules(&complete(vec![rule(&["*"], &["*"], &["*"])]));
    assert_eq!(r.granted, RBAC_DERIVED);
    assert_eq!(r.denied(), C::empty());
    assert_eq!(r.available(), RBAC_DERIVED);
}

#[test]
fn no_rules_and_complete_means_denied_not_unknown() {
    let r = capabilities_from_rules(&complete(vec![]));
    assert_eq!(r.granted, C::empty());
    assert_eq!(r.unknown, C::empty());
    assert_eq!(r.denied(), RBAC_DERIVED);
}

#[test]
fn read_only_user_can_read_logs_but_not_mutate_or_exec() {
    let r = capabilities_from_rules(&complete(vec![
        rule(&["*"], &["*"], &["get", "list", "watch"]),
        rule(&[""], &["pods/log"], &["get"]),
    ]));
    assert_eq!(r.granted, C::LOGS);
    assert_eq!(r.denied(), C::MUTATE | C::EXEC | C::PORTFORWARD);
}

#[test]
fn wildcard_resource_includes_subresources_but_exec_still_needs_create() {
    let r = capabilities_from_rules(&complete(vec![rule(&[""], &["*"], &["get"])]));
    assert!(r.granted.contains(C::LOGS), "`*` includes subresources");
    assert!(!r.granted.contains(C::EXEC), "exec needs create");
}

#[test]
fn view_role_without_log_subresource_has_no_logs() {
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["pods", "services"],
        &["get", "list", "watch"],
    )]));
    assert_eq!(r.granted, C::empty());
}

#[test]
fn exec_and_portforward_need_both_get_and_create() {
    let r = capabilities_from_rules(&complete(vec![
        rule(&[""], &["pods/exec"], &["get", "create"]),
        rule(&[""], &["pods/portforward"], &["create"]),
    ]));
    assert_eq!(r.granted, C::EXEC);
    assert!(
        r.denied().contains(C::PORTFORWARD),
        "create alone is not enough"
    );
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["pods/exec", "pods/portforward"],
        &["get"],
    )]));
    assert_eq!(r.granted, C::empty(), "get alone is not enough");
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["pods/portforward"],
        &["create", "get"],
    )]));
    assert_eq!(r.granted, C::PORTFORWARD);
}

#[test]
fn get_and_create_may_come_from_different_rules() {
    let r = capabilities_from_rules(&complete(vec![
        rule(&[""], &["pods/exec"], &["get"]),
        rule(&["*"], &["*"], &["create"]),
    ]));
    assert_eq!(r.granted, C::EXEC | C::MUTATE);
}

#[test]
fn a_restricted_half_makes_the_whole_requirement_restricted() {
    let r = capabilities_from_rules(&complete(vec![
        rule(&[""], &["pods/exec"], &["get"]),
        named(rule(&[""], &["pods/exec"], &["create"]), &["web-0"]),
    ]));
    assert!(r.restricted.contains(C::EXEC));
    assert!(!r.granted.contains(C::EXEC));
}

#[test]
fn star_slash_subresource_matches_any_resource() {
    let r = capabilities_from_rules(&complete(vec![rule(&[""], &["*/exec", "*/log"], &["*"])]));
    assert_eq!(r.granted, C::EXEC | C::LOGS);
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["*/exec"],
        &["get", "create"],
    )]));
    assert!(!r.granted.contains(C::LOGS));
}

#[test]
fn pods_slash_star_is_not_an_rbac_form() {
    let r = capabilities_from_rules(&complete(vec![rule(&[""], &["pods/*"], &["*"])]));
    assert_eq!(r.granted, C::empty());
}

#[test]
fn wildcard_verbs_apply_to_one_subresource_only() {
    let r = capabilities_from_rules(&complete(vec![rule(&[""], &["pods/exec"], &["*"])]));
    assert_eq!(r.granted, C::EXEC);
}

#[test]
fn api_group_must_match_pods_are_in_the_core_group() {
    let r = capabilities_from_rules(&complete(vec![rule(
        &["apps"],
        &["pods/exec", "pods/log"],
        &["*"],
    )]));
    assert_eq!(r.granted, C::empty());
    let r = capabilities_from_rules(&complete(vec![rule(
        &["apps", ""],
        &["pods/exec"],
        &["get", "create"],
    )]));
    assert_eq!(r.granted, C::EXEC);
    let r = capabilities_from_rules(&complete(vec![rule(&["*"], &["pods/log"], &["get"])]));
    assert_eq!(r.granted, C::LOGS);
}

#[test]
fn any_mutating_verb_on_any_object_resource_grants_mutate() {
    for verb in [
        "create",
        "update",
        "patch",
        "delete",
        "deletecollection",
        "*",
    ] {
        let r =
            capabilities_from_rules(&complete(vec![rule(&["apps"], &["deployments"], &[verb])]));
        assert!(r.granted.contains(C::MUTATE), "{verb}");
    }
    let r = capabilities_from_rules(&complete(vec![rule(
        &["apps"],
        &["deployments"],
        &["get", "list", "watch"],
    )]));
    assert!(!r.granted.contains(C::MUTATE));
}

#[test]
fn basic_user_review_rights_do_not_make_a_mutator() {
    // What `system:basic-user` grants every authenticated user.
    let basic_user = vec![
        rule(
            &["authorization.k8s.io"],
            &["selfsubjectaccessreviews", "selfsubjectrulesreviews"],
            &["create"],
        ),
        rule(
            &["authentication.k8s.io"],
            &["selfsubjectreviews"],
            &["create"],
        ),
    ];
    let r = capabilities_from_rules(&complete(basic_user));
    assert_eq!(r.granted, C::empty());
    assert_eq!(r.denied(), RBAC_DERIVED);
}

#[test]
fn stream_subresources_do_not_count_as_mutation() {
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["pods/exec", "pods/portforward", "pods/attach"],
        &["get", "create"],
    )]));
    assert!(!r.granted.contains(C::MUTATE));
    assert!(r.granted.contains(C::EXEC | C::PORTFORWARD));
    // A mixed resource list still counts: `configmaps` is an object resource.
    let r = capabilities_from_rules(&complete(vec![rule(
        &[""],
        &["pods/exec", "configmaps"],
        &["create"],
    )]));
    assert!(r.granted.contains(C::MUTATE));
}

#[test]
fn resource_names_restrict_instead_of_grant() {
    let r = capabilities_from_rules(&complete(vec![
        named(rule(&[""], &["pods/exec"], &["get", "create"]), &["web-0"]),
        named(
            rule(&[""], &["configmaps"], &["update", "patch"]),
            &["app-config"],
        ),
    ]));
    assert_eq!(r.granted, C::empty());
    assert_eq!(r.restricted, C::EXEC | C::MUTATE);
    assert_eq!(r.available(), C::EXEC | C::MUTATE);
    assert_eq!(r.level(Capability::Exec), AccessLevel::Restricted);
    assert_eq!(r.denied(), C::LOGS | C::PORTFORWARD);
}

#[test]
fn an_unrestricted_rule_beats_a_restricted_one() {
    let r = capabilities_from_rules(&complete(vec![
        named(rule(&[""], &["pods/exec"], &["get", "create"]), &["web-0"]),
        rule(&[""], &["pods/exec"], &["get", "create"]),
    ]));
    assert_eq!(r.granted, C::EXEC);
    assert_eq!(r.restricted, C::empty());
}

#[test]
fn incomplete_review_makes_unmatched_flags_unknown_not_denied() {
    let r = capabilities_from_rules(&partial(vec![rule(&[""], &["pods/log"], &["get"])]));
    assert_eq!(r.granted, C::LOGS);
    assert_eq!(r.unknown, C::MUTATE | C::EXEC | C::PORTFORWARD);
    assert_eq!(r.denied(), C::empty());
    assert_eq!(r.level(Capability::Mutate), AccessLevel::Unknown);
}

#[test]
fn evaluation_error_counts_as_incomplete() {
    let snap = RulesSnapshot {
        evaluation_error: Some("webhook timeout".into()),
        ..Default::default()
    };
    let r = capabilities_from_rules(&snap);
    assert_eq!(r.unknown, RBAC_DERIVED);
    assert_eq!(r.denied(), C::empty());
}

#[test]
fn incomplete_review_keeps_what_it_did_find_even_if_restricted() {
    let r = capabilities_from_rules(&partial(vec![named(
        rule(&[""], &["pods/exec"], &["get", "create"]),
        &["web-0"],
    )]));
    assert_eq!(r.restricted, C::EXEC);
    assert!(!r.unknown.contains(C::EXEC));
}

#[test]
fn levels_cover_flags_rbac_does_not_decide() {
    let r = capabilities_from_rules(&complete(vec![rule(&["*"], &["*"], &["*"])]));
    assert_eq!(r.level(Capability::Helm), AccessLevel::Unknown);
    assert_eq!(r.level(Capability::Metrics), AccessLevel::Unknown);
    assert_eq!(r.level(Capability::Logs), AccessLevel::Granted);
    let none = capabilities_from_rules(&complete(vec![]));
    assert_eq!(none.level(Capability::Logs), AccessLevel::Denied);
}

#[test]
fn empty_field_lists_match_nothing() {
    let r = capabilities_from_rules(&complete(vec![AccessRule {
        verbs: vec!["*".into()],
        ..Default::default()
    }]));
    assert_eq!(r.granted, C::empty());
}

#[test]
fn outcome_sets_are_disjoint_for_assorted_inputs() {
    let snaps = [
        complete(vec![]),
        partial(vec![]),
        complete(vec![rule(&["*"], &["*"], &["*"])]),
        partial(vec![named(
            rule(&[""], &["pods/exec", "pods/log"], &["*"]),
            &["a"],
        )]),
    ];
    for s in snaps {
        let r = capabilities_from_rules(&s);
        assert!((r.granted & r.restricted).is_empty());
        assert!((r.granted & r.unknown).is_empty());
        assert!((r.restricted & r.unknown).is_empty());
        assert_eq!(
            r.granted | r.restricted | r.unknown | r.denied(),
            RBAC_DERIVED
        );
    }
}
