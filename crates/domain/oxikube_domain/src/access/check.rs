//! [`AccessCheck`] and [`AccessDecision`]: one "may I?" question and its answer.
//!
//! The question a `SelfSubjectAccessReview` (or a cached rules review) answers: may the
//! current identity perform `verb` on a resource, optionally in a namespace and on one named
//! object. The answer has three outcomes, not two: an unreachable or partial authorizer is
//! [`Unknown`](AccessDecision::Unknown), never silently [`Denied`](AccessDecision::Denied), so
//! a soft gate fails open.

use super::{Access, AccessRules, is_cluster_scoped};
use crate::ids::Gvr;

/// Who a check is asked as, when not the current identity (`kubectl --as`).
///
/// Answered by a `SubjectAccessReview` (the can-i tool, E17-S08); an adapter that cannot ask as
/// another identity answers such a check with `Unsupported`. Lives in memory only: it is never
/// persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Impersonate {
    /// The user name (`--as`).
    pub user: String,
    /// Groups (`--as-group`).
    pub groups: Vec<String>,
}

impl Impersonate {
    /// A user (`--as <user>`).
    pub fn user(user: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            groups: Vec::new(),
        }
    }

    /// A group alone (`--as-group <group>`), with no user name.
    pub fn group(group: impl Into<String>) -> Self {
        Self {
            user: String::new(),
            groups: vec![group.into()],
        }
    }

    /// A ServiceAccount: the user `system:serviceaccount:<namespace>:<name>` in the groups the
    /// API server gives every ServiceAccount.
    pub fn service_account(namespace: &str, name: &str) -> Self {
        Self {
            user: format!("system:serviceaccount:{namespace}:{name}"),
            groups: vec![
                "system:serviceaccounts".to_owned(),
                format!("system:serviceaccounts:{namespace}"),
                "system:authenticated".to_owned(),
            ],
        }
    }
}

/// One action to check: `verb` on `gvr[/subresource]`, optionally scoped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccessCheck {
    /// `get`, `list`, `create`, `patch`, `delete`, ...
    pub verb: String,
    /// The resource kind (group, version, plural).
    pub gvr: Gvr,
    /// A subresource such as `exec` or `log`.
    pub subresource: Option<String>,
    /// The namespace; `None` for cluster-scoped checks or "all namespaces".
    pub namespace: Option<String>,
    /// A specific object name; `None` for the kind as a whole.
    pub name: Option<String>,
    /// Ask as another identity instead of the current one.
    pub impersonate: Option<Impersonate>,
}

impl AccessCheck {
    /// A check of `verb` on `gvr`, widened with the builder methods.
    pub fn new(verb: impl Into<String>, gvr: Gvr) -> Self {
        Self {
            verb: verb.into(),
            gvr,
            subresource: None,
            namespace: None,
            name: None,
            impersonate: None,
        }
    }

    /// Check a subresource (`exec`, `log`, `portforward`, ...).
    #[must_use]
    pub fn subresource(mut self, subresource: impl Into<String>) -> Self {
        self.subresource = Some(subresource.into());
        self
    }

    /// Check within a namespace.
    #[must_use]
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// Check one named object.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Ask as `identity` instead of the current user.
    #[must_use]
    pub fn impersonating(mut self, identity: Impersonate) -> Self {
        self.impersonate = Some(identity);
        self
    }

    /// The resource as RBAC rules spell it: `pods`, or `pods/exec` with a subresource.
    fn rule_resource(&self) -> String {
        match &self.subresource {
            Some(sub) => format!("{}/{sub}", self.gvr.resource),
            None => self.gvr.resource.to_string(),
        }
    }
}

/// The answer to an [`AccessCheck`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    /// The action is allowed.
    Allowed,
    /// The action is not allowed (explicitly denied, or no authorizer allowed it).
    Denied {
        /// The authorizer's reason, when it gave one.
        reason: Option<String>,
    },
    /// The authorizer could not decide (an evaluation error or a partial review, and no allow).
    Unknown {
        /// Why it could not.
        reason: String,
    },
}

impl AccessDecision {
    /// True only for [`Allowed`](Self::Allowed).
    pub fn is_allowed(&self) -> bool {
        matches!(self, AccessDecision::Allowed)
    }

    /// Whether a soft gate should still offer the action: everything but a clear deny.
    pub fn is_offered(&self) -> bool {
        !matches!(self, AccessDecision::Denied { .. })
    }
}

impl AccessRules {
    /// Answers `check` from the rules alone, or `None` when only the server can say.
    ///
    /// Decided: a grant on every object is `Allowed`; no matching rule in a complete review is
    /// `Denied`. Not decided: a partial review, a grant restricted to named objects (the
    /// object name may or may not be among them), a namespaced resource the review did not
    /// cover, and any impersonated check (the rules are the current identity's).
    pub fn decide(&self, check: &AccessCheck) -> Option<AccessDecision> {
        if check.impersonate.is_some() {
            return None;
        }
        let resource = check.rule_resource();
        if check.namespace.is_none()
            && self.namespaces_unseen
            && !is_cluster_scoped(&check.gvr.group, &resource)
        {
            // An all-namespaces ask on a namespaced resource: the review covered one probe
            // namespace, so even a matching rule may be that namespace's Role alone.
            return None;
        }
        let level = self.level(&check.verb, &check.gvr.group, &resource);
        match level {
            Access::Granted => Some(AccessDecision::Allowed),
            Access::Denied => Some(AccessDecision::Denied {
                reason: Some("no rule of the rules review allows it".to_owned()),
            }),
            Access::Restricted | Access::Unknown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessRule;

    fn pods() -> Gvr {
        Gvr::new("", "v1", "pods")
    }

    #[test]
    fn rules_decide_grants_and_complete_denials() {
        let rules = AccessRules::none().with_rule(AccessRule::granting(
            &["get", "list"],
            &[""],
            &["pods"],
            &[],
        ));
        assert_eq!(
            rules.decide(&AccessCheck::new("list", pods())),
            Some(AccessDecision::Allowed)
        );
        assert!(matches!(
            rules.decide(&AccessCheck::new("delete", pods())),
            Some(AccessDecision::Denied { .. })
        ));
    }

    #[test]
    fn an_all_namespaces_ask_is_not_answered_from_one_probe_namespace() {
        let unseen = AccessRules::none()
            .with_rule(AccessRule::granting(
                &["list"],
                &[""],
                &["pods", "nodes"],
                &[],
            ))
            .into_namespaces_unseen();
        assert_eq!(
            unseen.decide(&AccessCheck::new("list", pods())),
            None,
            "a Role of the probe namespace must not allow the all-namespaces ask"
        );
        assert_eq!(
            unseen.decide(&AccessCheck::new("list", pods()).namespace("default")),
            Some(AccessDecision::Allowed)
        );
        assert_eq!(
            unseen.decide(&AccessCheck::new("list", Gvr::new("", "v1", "nodes"))),
            Some(AccessDecision::Allowed),
            "cluster-scoped resources are fully covered by the review"
        );
    }

    #[test]
    fn impersonation_targets_spell_the_api_servers_identities() {
        assert_eq!(Impersonate::user("jane").groups, Vec::<String>::new());
        let group = Impersonate::group("devs");
        assert_eq!(
            (group.user.as_str(), group.groups),
            ("", vec!["devs".to_owned()])
        );
        let sa = Impersonate::service_account("shop", "bot");
        assert_eq!(sa.user, "system:serviceaccount:shop:bot");
        assert!(
            sa.groups
                .contains(&"system:serviceaccounts:shop".to_owned())
        );
    }

    #[test]
    fn subresources_are_matched_as_resource_slash_sub() {
        let rules = AccessRules::none().with_rule(AccessRule::granting(
            &["create"],
            &[""],
            &["pods/exec"],
            &[],
        ));
        let exec = AccessCheck::new("create", pods()).subresource("exec");
        assert_eq!(rules.decide(&exec), Some(AccessDecision::Allowed));
        assert!(matches!(
            rules.decide(&AccessCheck::new("create", pods())),
            Some(AccessDecision::Denied { .. })
        ));
    }

    #[test]
    fn rules_leave_the_uncertain_to_the_server() {
        let named = AccessRules::none().with_rule(AccessRule::granting(
            &["get"],
            &[""],
            &["pods"],
            &["web"],
        ));
        assert_eq!(
            named.decide(&AccessCheck::new("get", pods()).name("web")),
            None
        );
        let partial = AccessRules::none().into_partial();
        assert_eq!(partial.decide(&AccessCheck::new("get", pods())), None);
        let unseen = AccessRules::none().into_namespaces_unseen();
        assert_eq!(unseen.decide(&AccessCheck::new("get", pods())), None);
        let admin = AccessRules::all_access();
        let as_other = AccessCheck::new("get", pods()).impersonating(Impersonate {
            user: "bob".into(),
            groups: Vec::new(),
        });
        assert_eq!(admin.decide(&as_other), None, "rules are the caller's own");
    }

    #[test]
    fn only_a_clear_deny_is_not_offered() {
        assert!(AccessDecision::Allowed.is_offered());
        assert!(AccessDecision::Unknown { reason: "x".into() }.is_offered());
        assert!(!AccessDecision::Denied { reason: None }.is_offered());
    }
}
