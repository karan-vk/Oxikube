//! Properties of the normaliser over random rule sets: it answers every request exactly as the
//! rules as written do, and its output is canonical.

use proptest::prelude::*;
use proptest::sample::subsequence;

use super::super::{MATRIX_VERBS, NormalisedRules, PolicyMatrix, PolicyRule};
use crate::access::{AccessRules, non_resource_url_matches};

const VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update",
    "patch",
    "delete",
    "deletecollection",
    "bind",
    "*",
];
const GROUPS: &[&str] = &["", "apps", "batch", "*"];
const RESOURCES: &[&str] = &[
    "pods",
    "pods/log",
    "deployments",
    "deployments/scale",
    "*/scale",
    "*",
    "jobs",
];
const NAMES: &[&str] = &["a", "b"];
const URLS: &[&str] = &["/healthz", "/metrics", "/metrics/*", "/api/*", "/api*", "*"];

fn strings(pool: &'static [&'static str], max: usize) -> impl Strategy<Value = Vec<String>> {
    subsequence(pool, 0..=max).prop_map(|items| items.iter().map(|s| (*s).to_owned()).collect())
}

fn rule() -> impl Strategy<Value = PolicyRule> {
    prop_oneof![
        3 => (
            strings(VERBS, 3),
            strings(GROUPS, 2),
            strings(RESOURCES, 3),
            prop_oneof![2 => Just(Vec::new()), 1 => strings(NAMES, 2)],
        )
            .prop_map(|(verbs, api_groups, resources, resource_names)| PolicyRule {
                verbs,
                api_groups,
                resources,
                resource_names,
                non_resource_urls: Vec::new(),
            }),
        1 => (strings(VERBS, 2), strings(URLS, 3)).prop_map(|(verbs, non_resource_urls)| {
            PolicyRule { verbs, non_resource_urls, ..PolicyRule::default() }
        }),
    ]
}

/// One request: verb, group, resource, optional name, and a non-resource path.
fn request() -> impl Strategy<
    Value = (
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    ),
> {
    (
        prop::sample::select(&["get", "list", "delete", "bind", "use"][..]),
        prop::sample::select(&["", "apps", "batch", "x.io"][..]),
        prop::sample::select(
            &[
                "pods",
                "pods/log",
                "pods/exec",
                "deployments",
                "deployments/scale",
                "statefulsets/scale",
                "jobs",
            ][..],
        ),
        prop::option::of(prop::sample::select(&["a", "b", "c"][..])),
        prop::sample::select(
            &[
                "/healthz",
                "/metrics",
                "/metrics/x",
                "/api/v1",
                "/apis",
                "/",
            ][..],
        ),
    )
}

proptest! {
    #[test]
    fn normalising_never_changes_an_answer(
        rules in prop::collection::vec(rule(), 0..8),
        requests in prop::collection::vec(request(), 1..24),
    ) {
        let normalised = NormalisedRules::from_policy(&rules);
        let raw = AccessRules {
            rules: rules.iter().map(PolicyRule::to_access_rule).collect(),
            ..AccessRules::default()
        };
        for (verb, group, resource, name, path) in requests {
            prop_assert_eq!(
                normalised.level(verb, group, resource, name),
                raw.level_for(verb, group, resource, name),
                "{} {}/{} {:?}", verb, group, resource, name
            );
            let raw_url = rules.iter().any(|rule| {
                rule.non_resource_urls.iter().any(|url| non_resource_url_matches(url, path))
                    && rule.verbs.iter().any(|v| v == "*" || v == verb)
            });
            prop_assert_eq!(normalised.allows_url(verb, path), raw_url, "{} {}", verb, path);
        }
    }

    #[test]
    fn the_output_is_canonical(rules in prop::collection::vec(rule(), 0..8)) {
        let normalised = NormalisedRules::from_policy(&rules);
        let mut reversed = rules.clone();
        reversed.reverse();
        prop_assert_eq!(&NormalisedRules::from_policy(&reversed), &normalised, "order-independent");
        prop_assert_eq!(&NormalisedRules::union([&normalised]), &normalised, "idempotent");
        let doubled: Vec<PolicyRule> = rules.iter().chain(&rules).cloned().collect();
        prop_assert_eq!(&NormalisedRules::from_policy(&doubled), &normalised, "duplicates merge");
        for rule in normalised.resource_rules() {
            prop_assert!(rule.api_groups.len() == 1 && rule.resources.len() == 1);
            prop_assert!(rule.resource_names.len() <= 1);
            prop_assert!(rule.verbs.windows(2).all(|pair| pair[0] < pair[1]), "sorted, unique");
            prop_assert!(rule.verbs.len() == 1 || !rule.verbs.iter().any(|v| v == "*"));
        }
    }

    #[test]
    fn every_matrix_cell_is_what_the_whole_set_answers(
        rules in prop::collection::vec(rule(), 0..8),
    ) {
        let normalised = NormalisedRules::from_policy(&rules);
        let matrix = PolicyMatrix::from_rules(&normalised);
        for row in &matrix.rows {
            for (verb, cell) in MATRIX_VERBS.iter().zip(row.cells) {
                prop_assert_eq!(
                    cell,
                    normalised.level(verb.as_str(), &row.api_group, &row.resource, None),
                    "{} {}/{}", verb.as_str(), row.api_group, row.resource
                );
            }
        }
    }
}
