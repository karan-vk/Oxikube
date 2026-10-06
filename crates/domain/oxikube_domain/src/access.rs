//! What the current user may do, as RBAC rules: [`AccessRules`], [`Access`],
//! [`AccessRequirement`].
//!
//! A `SelfSubjectRulesReview` answers "what may I do in this namespace" as a list of allow
//! rules (RBAC has no deny rules). [`AccessRules`] is that list as plain data, so the sidebar
//! (and any other view that hides what the user cannot reach) can ask per-resource questions
//! without a Kubernetes type. The coarse session flags (`MUTATE`, `EXEC`, ...) stay in
//! [`Capabilities`](crate::Capabilities); this module answers the per-resource question
//! "may I `list` `deployments.apps`?".
//!
//! # Four answers, not two
//!
//! RBAC answers have more outcomes than yes and no ([`Access`]): granted on every object,
//! restricted to named objects (`resourceNames`), unknown (the review was partial, so more
//! rules may exist) and denied. Only [`Access::Denied`] hides anything: unknown is never
//! reported as denied.
//!
//! Matching follows the API server's RBAC rules: `*` matches any verb, API group or resource;
//! `*/exec` matches the `exec` subresource of any resource; the core group is the empty
//! string.

use serde::{Deserialize, Serialize};

use crate::kinds::Verb;

/// How the user stands on one action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// Not allowed (the review was complete and no rule matched).
    Denied,
    /// Nothing matched but the review was partial: more rules may exist.
    Unknown,
    /// Allowed only on objects named in `resourceNames` rules.
    Restricted,
    /// Allowed on every object.
    Granted,
}

impl Access {
    /// Whether the user should be offered the action: everything but [`Access::Denied`].
    pub fn is_offered(self) -> bool {
        self != Access::Denied
    }
}

/// One allow rule of a rules review (`ResourceRule`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRule {
    /// Verbs the rule allows; `*` is all.
    pub verbs: Vec<String>,
    /// API groups; `""` is the core group, `*` is all.
    pub api_groups: Vec<String>,
    /// Resources and subresources (`pods`, `pods/exec`); `*` is all, `*/exec` is any
    /// resource's `exec`.
    pub resources: Vec<String>,
    /// When non-empty, the rule applies only to objects with these names.
    pub resource_names: Vec<String>,
}

impl AccessRule {
    /// A single rule granting `verbs` on `resources` of `api_groups`, for tests and fakes.
    #[must_use]
    pub fn granting(
        verbs: &[&str],
        api_groups: &[&str],
        resources: &[&str],
        resource_names: &[&str],
    ) -> Self {
        let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        Self {
            verbs: own(verbs),
            api_groups: own(api_groups),
            resources: own(resources),
            resource_names: own(resource_names),
        }
    }

    /// Whether the rule covers `verb` on `group`/`resource`.
    fn covers(&self, verb: &str, group: &str, resource: &str) -> bool {
        self.api_groups.iter().any(|g| g == "*" || g == group)
            && self.resources.iter().any(|r| resource_matches(r, resource))
            && self.verbs.iter().any(|v| v == "*" || v == verb)
    }
}

/// RBAC resource matching for a requested `resource[/subresource]`.
fn resource_matches(rule: &str, requested: &str) -> bool {
    if rule == "*" || rule == requested {
        return true;
    }
    match (rule.strip_prefix("*/"), requested.split_once('/')) {
        (Some(sub), Some((_, requested_sub))) => sub == requested_sub,
        _ => false,
    }
}

/// The rules of one or more reviews, plus how trustworthy the list is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRules {
    /// Resource allow rules.
    pub rules: Vec<AccessRule>,
    /// The server could not list every rule (webhook or node authorizers) or reported an
    /// evaluation error: more rules may exist than are listed.
    pub partial: bool,
}

impl AccessRules {
    /// Rules with no grants and a complete review: everything is denied.
    pub fn none() -> Self {
        Self::default()
    }

    /// Rules that allow every verb on every resource (cluster admin).
    pub fn all_access() -> Self {
        Self {
            rules: vec![AccessRule::granting(&["*"], &["*"], &["*"], &[])],
            partial: false,
        }
    }

    /// The same list with one more rule.
    #[must_use]
    pub fn with_rule(mut self, rule: AccessRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// The same list marked partial.
    #[must_use]
    pub fn into_partial(mut self) -> Self {
        self.partial = true;
        self
    }

    /// Adds the rules of `other`: the union of what both reviews allow. Partial when either is.
    pub fn merge(&mut self, other: AccessRules) {
        self.rules.extend(other.rules);
        self.partial |= other.partial;
    }

    /// What the rules say about `verb` on `group`/`resource` (`resource` may carry a
    /// subresource: `pods/log`).
    pub fn level(&self, verb: &str, group: &str, resource: &str) -> Access {
        let mut best = None::<Access>;
        for rule in self
            .rules
            .iter()
            .filter(|r| r.covers(verb, group, resource))
        {
            if rule.resource_names.is_empty() {
                return Access::Granted;
            }
            best = Some(Access::Restricted);
        }
        match best {
            Some(level) => level,
            None if self.partial => Access::Unknown,
            None => Access::Denied,
        }
    }

    /// What the rules say about `requirement`.
    pub fn check(&self, requirement: &AccessRequirement) -> Access {
        self.level(
            requirement.verb.as_str(),
            &requirement.group,
            &requirement.resource,
        )
    }

    /// Whether any of `requirements` is offered (not denied). An empty list is always met:
    /// something that needs nothing is always visible.
    pub fn any_offered(&self, requirements: &[AccessRequirement]) -> bool {
        requirements.is_empty() || requirements.iter().any(|r| self.check(r).is_offered())
    }
}

/// What a UI entry needs the user to be able to do before it is shown: one verb on one
/// resource (usually `list`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccessRequirement {
    /// API group of the resource (`""` is the core group).
    pub group: String,
    /// Plural resource name (`deployments`).
    pub resource: String,
    /// The verb that must be allowed.
    pub verb: Verb,
}

impl AccessRequirement {
    /// `list` on `resource` of `group`.
    pub fn list(group: impl Into<String>, resource: impl Into<String>) -> Self {
        Self {
            group: group.into(),
            resource: resource.into(),
            verb: Verb::List,
        }
    }
}

/// Whether `group` is one of the API groups Kubernetes itself serves, as opposed to a group a
/// CustomResourceDefinition (or an aggregated API) adds.
///
/// Built-in groups are the core group, the dot-less legacy groups (`apps`, `batch`,
/// `autoscaling`, `policy`, `extensions`) and the `*.k8s.io` groups the API server ships. It is
/// a heuristic over discovery output, which does not say which kinds come from CRDs; a CRD
/// group that is spelled like a built-in one (rare) shows as built-in.
pub fn is_builtin_api_group(group: &str) -> bool {
    const BUILTIN: [&str; 15] = [
        "admissionregistration.k8s.io",
        "apiextensions.k8s.io",
        "apiregistration.k8s.io",
        "authentication.k8s.io",
        "authorization.k8s.io",
        "certificates.k8s.io",
        "coordination.k8s.io",
        "discovery.k8s.io",
        "events.k8s.io",
        "flowcontrol.apiserver.k8s.io",
        "networking.k8s.io",
        "node.k8s.io",
        "rbac.authorization.k8s.io",
        "scheduling.k8s.io",
        "storage.k8s.io",
    ];
    group.is_empty()
        || !group.contains('.')
        || BUILTIN.contains(&group)
        || matches!(
            group,
            "metrics.k8s.io"
                | "custom.metrics.k8s.io"
                | "external.metrics.k8s.io"
                | "resource.k8s.io"
                | "internal.apiserver.k8s.io"
                | "storagemigration.k8s.io"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(rule: AccessRule) -> AccessRules {
        AccessRules::none().with_rule(rule)
    }

    #[test]
    fn nothing_matches_means_denied_unless_the_review_was_partial() {
        let pods = AccessRule::granting(&["list"], &[""], &["pods"], &[]);
        let r = rules(pods);
        assert_eq!(r.level("list", "", "pods"), Access::Granted);
        assert_eq!(r.level("list", "", "secrets"), Access::Denied);
        assert_eq!(r.level("delete", "", "pods"), Access::Denied);
        assert_eq!(r.level("list", "apps", "pods"), Access::Denied);
        let partial = r.into_partial();
        assert_eq!(partial.level("list", "", "secrets"), Access::Unknown);
        assert_eq!(partial.level("list", "", "pods"), Access::Granted);
        assert!(Access::Unknown.is_offered() && !Access::Denied.is_offered());
    }

    #[test]
    fn wildcards_and_subresources_follow_rbac() {
        let admin = AccessRules::all_access();
        assert_eq!(admin.level("list", "apps", "deployments"), Access::Granted);
        assert_eq!(admin.level("get", "", "pods/log"), Access::Granted);
        let any_exec = rules(AccessRule::granting(&["create"], &[""], &["*/exec"], &[]));
        assert_eq!(any_exec.level("create", "", "pods/exec"), Access::Granted);
        assert_eq!(any_exec.level("create", "", "pods"), Access::Denied);
        // `pods` alone does not grant its subresources.
        let pods = rules(AccessRule::granting(&["get"], &[""], &["pods"], &[]));
        assert_eq!(pods.level("get", "", "pods/log"), Access::Denied);
    }

    #[test]
    fn named_objects_only_is_restricted_and_full_grants_win() {
        let named = rules(AccessRule::granting(
            &["list"],
            &["apps"],
            &["deployments"],
            &["web"],
        ));
        assert_eq!(
            named.level("list", "apps", "deployments"),
            Access::Restricted
        );
        let both = named.with_rule(AccessRule::granting(
            &["list"],
            &["apps"],
            &["deployments"],
            &[],
        ));
        assert_eq!(both.level("list", "apps", "deployments"), Access::Granted);
    }

    #[test]
    fn merging_reviews_is_a_union_and_keeps_partiality() {
        let mut a = rules(AccessRule::granting(&["list"], &[""], &["pods"], &[]));
        let b = rules(AccessRule::granting(&["list"], &[""], &["secrets"], &[])).into_partial();
        a.merge(b);
        assert!(a.partial);
        assert_eq!(a.level("list", "", "pods"), Access::Granted);
        assert_eq!(a.level("list", "", "secrets"), Access::Granted);
        assert_eq!(a.level("list", "", "nodes"), Access::Unknown);
    }

    #[test]
    fn a_requirement_set_is_met_by_any_listable_member() {
        let r = rules(AccessRule::granting(
            &["list"],
            &["apps"],
            &["deployments"],
            &[],
        ));
        let reqs = [
            AccessRequirement::list("", "pods"),
            AccessRequirement::list("apps", "deployments"),
        ];
        assert!(r.any_offered(&reqs));
        assert!(!r.any_offered(&reqs[..1]));
        assert!(r.any_offered(&[]), "no requirement: always shown");
        assert_eq!(r.check(&reqs[1]), Access::Granted);
    }

    #[test]
    fn builtin_groups_are_told_from_custom_ones() {
        for builtin in [
            "",
            "apps",
            "batch",
            "networking.k8s.io",
            "rbac.authorization.k8s.io",
            "metrics.k8s.io",
        ] {
            assert!(is_builtin_api_group(builtin), "{builtin}");
        }
        for custom in [
            "argoproj.io",
            "cert-manager.io",
            "gateway.networking.k8s.io",
            "monitoring.coreos.com",
        ] {
            assert!(!is_builtin_api_group(custom), "{custom}");
        }
    }

    #[test]
    fn rules_round_trip_through_json() {
        let r = AccessRules::all_access().into_partial();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<AccessRules>(&json).unwrap(), r);
    }
}
