//! Table-driven normaliser cases.

use super::super::{NonResourceRule, NormalisedRule, NormalisedRules, PolicyRule};
use crate::access::{Access, AccessRule};

fn resource(verbs: &[&str], group: &str, resource: &str, name: Option<&str>) -> NormalisedRule {
    NormalisedRule::Resource(AccessRule::granting(
        verbs,
        &[group],
        &[resource],
        &name.into_iter().collect::<Vec<_>>(),
    ))
}

fn url(verbs: &[&str], url: &str) -> NormalisedRule {
    NormalisedRule::NonResource(NonResourceRule {
        url: url.to_owned(),
        verbs: verbs.iter().map(|v| (*v).to_owned()).collect(),
    })
}

fn normalise(rules: &[PolicyRule]) -> Vec<NormalisedRule> {
    NormalisedRules::from_policy(rules).rules().to_vec()
}

#[test]
fn rules_break_down_per_group_resource_and_name() {
    let cases: Vec<(&str, Vec<PolicyRule>, Vec<NormalisedRule>)> = vec![
        (
            "two groups by two resources",
            vec![PolicyRule::resource(
                &["get"],
                &["", "apps"],
                &["pods", "deployments"],
                &[],
            )],
            vec![
                resource(&["get"], "", "deployments", None),
                resource(&["get"], "", "pods", None),
                resource(&["get"], "apps", "deployments", None),
                resource(&["get"], "apps", "pods", None),
            ],
        ),
        (
            "resourceNames stay a restriction, one entry per name",
            vec![PolicyRule::resource(
                &["get", "update"],
                &[""],
                &["configmaps"],
                &["b", "a"],
            )],
            vec![
                resource(&["get", "update"], "", "configmaps", Some("a")),
                resource(&["get", "update"], "", "configmaps", Some("b")),
            ],
        ),
        (
            "subresources stay whole: pods/log and */scale",
            vec![
                PolicyRule::resource(&["get"], &[""], &["pods/log"], &[]),
                PolicyRule::resource(&["update"], &["apps"], &["*/scale"], &[]),
            ],
            vec![
                resource(&["get"], "", "pods/log", None),
                resource(&["update"], "apps", "*/scale", None),
            ],
        ),
        (
            "non-resource URLs are kept apart, sorted",
            vec![PolicyRule::non_resource(
                &["get"],
                &["/metrics/*", "/healthz"],
            )],
            vec![url(&["get"], "/healthz"), url(&["get"], "/metrics/*")],
        ),
        (
            "a rule without verbs or groups grants nothing",
            vec![
                PolicyRule::resource(&[], &[""], &["pods"], &[]),
                PolicyRule::resource(&["get"], &[], &["pods"], &[]),
            ],
            vec![],
        ),
    ];
    for (case, input, expected) in cases {
        assert_eq!(normalise(&input), expected, "{case}");
    }
}

#[test]
fn duplicates_merge_and_wildcard_verbs_absorb_the_rest() {
    let cases: Vec<(&str, Vec<PolicyRule>, Vec<NormalisedRule>)> = vec![
        (
            "duplicate verbs and duplicate rules",
            vec![
                PolicyRule::resource(&["list", "get", "get"], &["apps"], &["deployments"], &[]),
                PolicyRule::resource(&["watch", "list"], &["apps"], &["deployments"], &[]),
            ],
            vec![resource(
                &["get", "list", "watch"],
                "apps",
                "deployments",
                None,
            )],
        ),
        (
            "* verbs",
            vec![
                PolicyRule::resource(&["get"], &[""], &["configmaps"], &[]),
                PolicyRule::resource(&["*", "delete"], &[""], &["configmaps"], &[]),
            ],
            vec![resource(&["*"], "", "configmaps", None)],
        ),
        (
            "duplicate URLs",
            vec![
                PolicyRule::non_resource(&["get"], &["/healthz"]),
                PolicyRule::non_resource(&["get", "post"], &["/healthz"]),
            ],
            vec![url(&["get", "post"], "/healthz")],
        ),
    ];
    for (case, input, expected) in cases {
        assert_eq!(normalise(&input), expected, "{case}");
    }
}

#[test]
fn what_a_wider_rule_grants_is_pruned() {
    let cases: Vec<(&str, Vec<PolicyRule>, Vec<NormalisedRule>)> = vec![
        (
            "* resources in a group cover its resources",
            vec![
                PolicyRule::resource(&["get", "delete"], &["apps"], &["deployments"], &[]),
                PolicyRule::resource(&["get"], &["apps"], &["*"], &[]),
            ],
            vec![
                resource(&["get"], "apps", "*", None),
                resource(&["delete"], "apps", "deployments", None),
            ],
        ),
        (
            "* groups cover every group",
            vec![
                PolicyRule::resource(&["list"], &["*"], &["jobs"], &[]),
                PolicyRule::resource(&["list", "get"], &["batch"], &["jobs"], &[]),
            ],
            vec![
                resource(&["list"], "*", "jobs", None),
                resource(&["get"], "batch", "jobs", None),
            ],
        ),
        (
            "cluster-admin covers everything",
            vec![
                PolicyRule::resource(&["*"], &["*"], &["*"], &[]),
                PolicyRule::resource(&["get"], &[""], &["pods/log"], &["a"]),
                PolicyRule::resource(&["create"], &["apps"], &["*/scale"], &[]),
            ],
            vec![resource(&["*"], "*", "*", None)],
        ),
        (
            "*/scale covers deployments/scale but not deployments",
            vec![
                PolicyRule::resource(&["get"], &["apps"], &["*/scale"], &[]),
                PolicyRule::resource(&["get"], &["apps"], &["deployments/scale"], &[]),
                PolicyRule::resource(&["get"], &["apps"], &["deployments"], &[]),
            ],
            vec![
                resource(&["get"], "apps", "*/scale", None),
                resource(&["get"], "apps", "deployments", None),
            ],
        ),
        (
            "an unnamed rule covers the named one",
            vec![
                PolicyRule::resource(&["get", "update"], &[""], &["secrets"], &["s"]),
                PolicyRule::resource(&["get"], &[""], &["secrets"], &[]),
            ],
            vec![
                resource(&["get"], "", "secrets", None),
                resource(&["update"], "", "secrets", Some("s")),
            ],
        ),
        (
            "URL prefix globs cover what they match",
            vec![
                PolicyRule::non_resource(&["get"], &["/metrics/*", "/metrics/cadvisor"]),
                PolicyRule::non_resource(&["get"], &["/metrics"]),
            ],
            vec![url(&["get"], "/metrics"), url(&["get"], "/metrics/*")],
        ),
        (
            "* URL covers every path",
            vec![PolicyRule::non_resource(
                &["get"],
                &["*", "/healthz", "/api/*"],
            )],
            vec![url(&["get"], "*")],
        ),
    ];
    for (case, input, expected) in cases {
        assert_eq!(normalise(&input), expected, "{case}");
    }
}

#[test]
fn equal_policies_compare_equal_however_they_are_split() {
    let one = NormalisedRules::from_policy(&[PolicyRule::resource(
        &["get", "list"],
        &["", "apps"],
        &["pods", "deployments"],
        &[],
    )]);
    let split = NormalisedRules::from_policy(&[
        PolicyRule::resource(&["list"], &["apps"], &["deployments", "pods"], &[]),
        PolicyRule::resource(&["get"], &["apps", ""], &["pods", "deployments"], &[]),
        PolicyRule::resource(&["list"], &[""], &["deployments", "pods"], &[]),
    ]);
    assert_eq!(one, split);
    assert_eq!(NormalisedRules::union([&one, &split]), one);
}

#[test]
fn the_set_answers_through_the_access_matcher() {
    let rules = NormalisedRules::from_policy(&[
        PolicyRule::resource(&["get"], &[""], &["pods", "pods/log"], &[]),
        PolicyRule::resource(&["get"], &[""], &["secrets"], &["a"]),
        PolicyRule::resource(&["update"], &["apps"], &["*/scale"], &[]),
        PolicyRule::non_resource(&["get"], &["/metrics/*"]),
    ]);
    assert_eq!(rules.level("get", "", "pods/log", None), Access::Granted);
    assert_eq!(rules.level("get", "", "pods/exec", None), Access::Denied);
    assert_eq!(rules.level("get", "", "secrets", None), Access::Restricted);
    assert_eq!(
        rules.level("get", "", "secrets", Some("a")),
        Access::Granted
    );
    assert_eq!(rules.level("get", "", "secrets", Some("b")), Access::Denied);
    assert_eq!(
        rules.level("update", "apps", "statefulsets/scale", None),
        Access::Granted
    );
    assert_eq!(
        rules.level("update", "apps", "statefulsets", None),
        Access::Denied
    );
    assert!(rules.allows_url("get", "/metrics/cadvisor"));
    assert!(!rules.allows_url("get", "/metrics"));
    assert!(!rules.allows_url("post", "/metrics/cadvisor"));
    let access = rules.to_access_rules();
    assert_eq!(access.level("get", "", "pods"), Access::Granted);
    assert!(!access.partial);
    assert_eq!(
        rules.resource_rules().count() + rules.non_resource_rules().count(),
        rules.len()
    );
}
