//! Pure reduction of RBAC rules to the session [`Capabilities`] flags.
//!
//! A `SelfSubjectRulesReview` answers "what may I do in this namespace" as a list of
//! allow rules (RBAC has no deny rules). This module turns that list into the four flags
//! RBAC can decide: [`MUTATE`](Capabilities::MUTATE), [`EXEC`](Capabilities::EXEC),
//! [`LOGS`](Capabilities::LOGS) and [`PORTFORWARD`](Capabilities::PORTFORWARD). The other
//! flags (Helm, Argo, metrics) come from discovery and integrations, not from RBAC.
//!
//! The result is not a plain set, because RBAC answers have more than two outcomes:
//!
//! * **granted**: some rule grants the action on every object of the kind;
//! * **restricted**: only rules limited by `resourceNames` match (for example `exec` into
//!   one named pod). The action should be offered; a per-object [`can_i`](super::can_i)
//!   decides each target;
//! * **unknown**: nothing matched but the review was `incomplete` or had an
//!   `evaluationError`, so more rules may exist. Unknown is never reported as denied;
//! * **denied**: nothing matched and the review was complete.
//!
//! `EXEC` and `PORTFORWARD` need both `get` and `create` on their subresource (kube-rs
//! connects with a WebSocket upgrade, authorized as `get`; newer apiservers also check
//! `create`); `LOGS` needs `get` on `pods/log`.
//!
//! Matching follows the apiserver's RBAC rules: `*` matches any verb, API group or
//! resource; `*/exec` matches the `exec` subresource of any resource; the core group is
//! the empty string. (RBAC has no `pods/*` form.) Non-resource rules (`/healthz`, ...)
//! never grant a flag here.

use oxikube_domain::access::{AccessRule as DomainRule, AccessRules};
use oxikube_domain::{Capabilities, Capability};

/// One allow rule, as returned by a rules review (`ResourceRule`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// The rules of one review plus how trustworthy the list is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RulesSnapshot {
    /// Resource allow rules for the namespace.
    pub rules: Vec<AccessRule>,
    /// The server could not list every rule (webhook or node authorizers): more may exist.
    pub incomplete: bool,
    /// The authorizer reported an error while evaluating; treated like `incomplete`.
    pub evaluation_error: Option<String>,
}

impl RulesSnapshot {
    /// The rules as the domain's per-resource [`AccessRules`] (what the sidebar asks).
    pub fn to_access_rules(&self) -> AccessRules {
        AccessRules {
            rules: self
                .rules
                .iter()
                .map(|r| DomainRule {
                    verbs: r.verbs.clone(),
                    api_groups: r.api_groups.clone(),
                    resources: r.resources.clone(),
                    resource_names: r.resource_names.clone(),
                })
                .collect(),
            partial: self.is_partial(),
        }
    }

    /// True when the rule list may be missing rules.
    pub fn is_partial(&self) -> bool {
        self.incomplete || self.evaluation_error.is_some()
    }
}

/// What the RBAC rules say about each RBAC-derived capability.
///
/// The three sets are disjoint; whatever is in [`RBAC_DERIVED`] but in none of them is
/// [`denied`](Self::denied).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilityReport {
    /// Allowed on every object of the kind.
    pub granted: Capabilities,
    /// Allowed only on objects named in `resourceNames` rules.
    pub restricted: Capabilities,
    /// Not matched, but the review was partial, so it is not known to be denied.
    pub unknown: Capabilities,
}

/// The capabilities a rules review can decide.
pub const RBAC_DERIVED: Capabilities = Capabilities::MUTATE
    .union(Capabilities::EXEC)
    .union(Capabilities::LOGS)
    .union(Capabilities::PORTFORWARD);

/// How the user stands on one capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    /// Allowed on every object.
    Granted,
    /// Allowed on named objects only.
    Restricted,
    /// Could not be determined (incomplete review).
    Unknown,
    /// Not allowed.
    Denied,
}

impl CapabilityReport {
    /// RBAC-derived capabilities that are definitely not available.
    pub fn denied(&self) -> Capabilities {
        RBAC_DERIVED - self.granted - self.restricted - self.unknown
    }

    /// Capabilities to offer in the UI: granted plus restricted. A restricted action may
    /// still be forbidden on a particular object; check it with [`can_i`](super::can_i).
    pub fn available(&self) -> Capabilities {
        self.granted | self.restricted
    }

    /// The level of one capability. Flags that RBAC does not decide report `Unknown`.
    pub fn level(&self, capability: Capability) -> AccessLevel {
        let flag = capability.flag();
        if !RBAC_DERIVED.contains(flag) {
            AccessLevel::Unknown
        } else if self.granted.contains(flag) {
            AccessLevel::Granted
        } else if self.restricted.contains(flag) {
            AccessLevel::Restricted
        } else if self.unknown.contains(flag) {
            AccessLevel::Unknown
        } else {
            AccessLevel::Denied
        }
    }
}

/// Reduces a rules review to capability levels.
pub fn capabilities_from_rules(snapshot: &RulesSnapshot) -> CapabilityReport {
    let rules = &snapshot.rules;
    let levels = [
        (Capabilities::MUTATE, mutate_grant(rules)),
        (
            Capabilities::EXEC,
            requirement_grant(rules, "", "pods/exec", &STREAM_VERBS),
        ),
        (
            Capabilities::LOGS,
            requirement_grant(rules, "", "pods/log", &["get"]),
        ),
        (
            Capabilities::PORTFORWARD,
            requirement_grant(rules, "", "pods/portforward", &STREAM_VERBS),
        ),
    ];
    let mut report = CapabilityReport::default();
    for (flag, grant) in levels {
        match grant {
            Grant::Full => report.granted |= flag,
            Grant::Restricted => report.restricted |= flag,
            Grant::None if snapshot.is_partial() => report.unknown |= flag,
            Grant::None => {}
        }
    }
    report
}

/// How much of a requirement the rules cover. Ordered: `Full` beats `Restricted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Grant {
    None,
    Restricted,
    Full,
}

impl Grant {
    fn of_rule(rule: &AccessRule) -> Grant {
        if rule.resource_names.is_empty() {
            Grant::Full
        } else {
            Grant::Restricted
        }
    }
}

/// Verbs a stream subresource (`exec`, `portforward`) needs. kube-rs opens these with a
/// WebSocket upgrade, which the apiserver authorizes as `get`; newer apiservers also
/// authorize the `create` of the equivalent POST path. Both are required, so a user
/// who can only do one of them is not offered an action that would end in a 403.
const STREAM_VERBS: [&str; 2] = ["get", "create"];

/// Best grant for one (group, resource) requirement: for each verb the best matching
/// rule, then the weakest verb. All verbs must be covered for anything above `None`.
fn requirement_grant(rules: &[AccessRule], group: &str, resource: &str, verbs: &[&str]) -> Grant {
    verbs
        .iter()
        .map(|verb| verb_grant(rules, group, resource, verb))
        .min()
        .unwrap_or(Grant::None)
}

fn verb_grant(rules: &[AccessRule], group: &str, resource: &str, verb: &str) -> Grant {
    rules
        .iter()
        .filter(|r| {
            any_matches(&r.api_groups, |g| g == "*" || g == group)
                && any_matches(&r.resources, |x| resource_matches(x, resource))
                && any_matches(&r.verbs, |v| v == "*" || v == verb)
        })
        .map(Grant::of_rule)
        .max()
        .unwrap_or(Grant::None)
}

fn any_matches(items: &[String], f: impl Fn(&str) -> bool) -> bool {
    items.iter().any(|i| f(i))
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

/// API groups that only hold review/identity resources every user may create.
const REVIEW_GROUPS: [&str; 2] = ["authorization.k8s.io", "authentication.k8s.io"];

/// Verbs that change cluster objects.
const MUTATING_VERBS: [&str; 6] = [
    "*",
    "create",
    "update",
    "patch",
    "delete",
    "deletecollection",
];

/// `MUTATE`: some mutating verb on something that is not just a review or a connection
/// subresource.
///
/// Every authenticated user may `create` `selfsubjectaccessreviews`, and `create` on
/// `pods/exec` or `pods/portforward` opens a stream rather than changing an object, so
/// neither makes a read-only user a mutator (see [`is_object_resource`]).
fn mutate_grant(rules: &[AccessRule]) -> Grant {
    rules
        .iter()
        .filter(|r| any_matches(&r.verbs, |v| MUTATING_VERBS.contains(&v)))
        .filter(|r| {
            !r.api_groups.is_empty()
                && !r
                    .api_groups
                    .iter()
                    .all(|g| REVIEW_GROUPS.contains(&g.as_str()))
        })
        .filter(|r| any_matches(&r.resources, is_object_resource))
        .map(Grant::of_rule)
        .max()
        .unwrap_or(Grant::None)
}

/// A resource entry that denotes stored objects rather than a read-only or stream
/// subresource (`log` only supports `get`; `exec`, `attach` and `portforward` open
/// streams). `x/*` is not a form RBAC matches, so it counts for nothing.
fn is_object_resource(resource: &str) -> bool {
    resource == "*"
        || !(resource.ends_with("/*")
            || ["/exec", "/attach", "/portforward", "/log"]
                .iter()
                .any(|s| resource.ends_with(s)))
}

#[cfg(test)]
mod tests;
