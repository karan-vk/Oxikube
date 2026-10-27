//! The index against what a kind cluster answered (`oxikube_testkit::fixtures::rbac`): the
//! rules review behind `kubectl auth can-i --list --as=<subject>`, the rows kubectl printed,
//! and who the API server took each subject for. Covers the built-in `view`, `edit`, `admin`
//! and `cluster-admin`, a custom aggregated ClusterRole, wildcards, `resourceNames`,
//! subresources, non-resource URLs and a ServiceAccount's implicit groups.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::{NormalisedRules, PolicyRule, Role, aggregated_rules};
use oxikube_testkit::fixtures::rbac::{self, RecordedReview};
use proptest::prelude::*;
use serde_json::Value;

use super::index_of;
use crate::access::SubjectIndex;

fn rules_of(status: &Value, key: &str) -> Vec<PolicyRule> {
    serde_json::from_value(status[key].clone()).expect("review rules are PolicyRules")
}

/// The review's rules as written by the API server, unnormalised.
fn review_rules(review: &RecordedReview) -> Vec<PolicyRule> {
    let mut rules = rules_of(&review.status, "resourceRules");
    rules.extend(rules_of(&review.status, "nonResourceRules"));
    rules
}

/// One row `kubectl auth can-i --list` printed, back as a rule: `resource.group/sub`,
/// `[urls]`, `[names]`, `[verbs]`.
fn printed_rule(line: &str) -> PolicyRule {
    let column = line[..line.find('[').expect("a printed row has lists")].trim();
    let lists: Vec<Vec<&str>> = line
        .split('[')
        .skip(1)
        .map(|part| {
            part.split(']')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect()
        })
        .collect();
    let [urls, names, verbs] = lists.as_slice() else {
        panic!("three lists in {line:?}");
    };
    if column.is_empty() {
        return PolicyRule::non_resource(verbs, urls);
    }
    let (head, subresource) = column
        .split_once('/')
        .map_or((column, None), |(h, s)| (h, Some(s)));
    let (resource, group) = head.split_once('.').unwrap_or((head, ""));
    let resource =
        subresource.map_or_else(|| resource.to_owned(), |sub| format!("{resource}/{sub}"));
    PolicyRule::resource(verbs, &[group], &[&resource], names)
}

fn recorded() -> (SubjectIndex, Vec<RecordedReview>) {
    (index_of(rbac::cluster()), rbac::reviews())
}

#[test]
fn subjects_expand_to_what_the_api_server_saw() {
    for review in rbac::reviews() {
        let info = review.subject.user_info();
        let groups: BTreeSet<&str> = info.groups.iter().map(|g| &**g).collect();
        let seen: BTreeSet<&str> = review.groups.iter().map(String::as_str).collect();
        assert_eq!(groups, seen, "{}", review.subject);
        if let Some(user) = info.user {
            assert_eq!(&*user, review.user, "{}", review.subject);
        }
    }
}

#[test]
fn the_index_lists_what_the_rules_review_lists() {
    let (index, reviews) = recorded();
    for review in &reviews {
        let ours = index.rules_in(&review.subject, Some(&review.namespace));
        let theirs = NormalisedRules::from_policy(&review_rules(review));
        assert_eq!(ours, theirs, "{} in {}", review.subject, review.namespace);
    }
}

#[test]
fn the_index_lists_what_kubectl_auth_can_i_list_printed() {
    let (index, reviews) = recorded();
    for review in &reviews {
        let printed: Vec<PolicyRule> = review.can_i_list.iter().map(|l| printed_rule(l)).collect();
        assert_eq!(
            index.rules_in(&review.subject, Some(&review.namespace)),
            NormalisedRules::from_policy(&printed),
            "{} in {}",
            review.subject,
            review.namespace
        );
    }
}

#[test]
fn aggregated_roles_resolve_to_their_live_rules_from_selectors_alone() {
    let roles: Vec<Role> = rbac::cluster()
        .iter()
        .filter_map(|o| Role::from_resource(o).ok())
        .collect();
    // Every aggregated ClusterRole with its live rules dropped: only the selectors remain.
    let stripped: Vec<Role> = roles
        .iter()
        .map(|role| match role.aggregation {
            Some(_) => Role {
                rules: Vec::new(),
                ..role.clone()
            },
            None => role.clone(),
        })
        .collect();
    let stripped_refs: Vec<&Role> = stripped.iter().collect();
    let mut checked = Vec::new();
    for (live, bare) in roles.iter().zip(&stripped) {
        if live.aggregation.is_none() {
            continue;
        }
        assert_eq!(
            aggregated_rules(bare, &stripped_refs),
            live.normalised(),
            "{}: the selectors give what the controller filled in",
            live.id
        );
        checked.push(live.id.name.to_string());
    }
    checked.sort();
    assert_eq!(checked, ["admin", "edit", "oxi-rbac-fx-aggregate", "view"]);
}

/// A request vocabulary drawn from the recorded rules, plus resources nobody is granted.
fn request() -> impl Strategy<
    Value = (
        usize,
        &'static str,
        (&'static str, &'static str),
        Option<&'static str>,
    ),
> {
    (
        0..12usize,
        prop::sample::select(
            &[
                "get",
                "list",
                "watch",
                "create",
                "update",
                "patch",
                "delete",
                "deletecollection",
                "bind",
                "impersonate",
            ][..],
        ),
        prop::sample::select(
            &[
                ("", "pods"),
                ("", "pods/log"),
                ("", "pods/exec"),
                ("", "configmaps"),
                ("", "secrets"),
                ("", "events"),
                ("", "serviceaccounts/token"),
                ("", "nodes"),
                ("apps", "deployments"),
                ("apps", "deployments/scale"),
                ("apps", "statefulsets/scale"),
                ("batch", "jobs"),
                ("batch", "cronjobs"),
                ("x.io", "jobs"),
                ("events.k8s.io", "events"),
                ("rbac.authorization.k8s.io", "roles"),
                ("authorization.k8s.io", "selfsubjectrulesreviews"),
                ("certificates.k8s.io", "clustertrustbundles"),
                ("metrics.k8s.io", "pods"),
            ][..],
        ),
        prop::option::of(prop::sample::select(
            &["fx-secret", "team-settings", "team-flags", "other"][..],
        )),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn can_matches_the_recorded_review_for_any_request(requests in prop::collection::vec(request(), 1..32)) {
        let (index, reviews) = recorded();
        let oracles: Vec<AccessRules> = reviews
            .iter()
            .map(|review| AccessRules {
                rules: review_rules(review).iter().map(PolicyRule::to_access_rule).collect(),
                ..AccessRules::default()
            })
            .collect();
        for (at, verb, (group, resource), name) in requests {
            let review = &reviews[at];
            let gvr = Gvr::new(Arc::<str>::from(group), "v1", resource);
            prop_assert_eq!(
                index.can(&review.subject, verb, &gvr, Some(&review.namespace), name),
                oracles[at].level_for(verb, group, resource, name),
                "{} {} {}/{} {:?} in {}", review.subject, verb, group, resource, name, review.namespace
            );
        }
    }
}
