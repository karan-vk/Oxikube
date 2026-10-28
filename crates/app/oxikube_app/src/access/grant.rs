//! [`Grant`] and [`ScopedRules`]: what the [`SubjectIndex`](super::SubjectIndex) answers with.

use std::sync::Arc;

use oxikube_domain::rbac::{BindingRef, GrantScope, NormalisedRules, RoleRef, Subject};

/// One binding's grant to a subject: which binding, which role, where it holds and the
/// normalised rules it gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The binding.
    pub binding: BindingRef,
    /// The binding's subject entry that matched: the subject itself, or a group it is in
    /// (`system:authenticated`, `system:serviceaccounts:<ns>`, ...).
    pub via: Subject,
    /// The role the binding names.
    pub role: RoleRef,
    /// Where the grant holds.
    pub scope: GrantScope,
    /// The role's rules, aggregation included; a namespaced grant carries no non-resource URLs
    /// (they are never namespaced, so a RoleBinding cannot grant them). Shared with the index's
    /// per-role cache.
    pub rules: Arc<NormalisedRules>,
    /// Whether the role exists. A binding to a missing role grants nothing (yet).
    pub role_found: bool,
}

/// A subject's rules where they hold: one entry per scope, the grants of that scope merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedRules {
    /// Where the rules hold.
    pub scope: GrantScope,
    /// The normalised union of every grant with this scope.
    pub rules: NormalisedRules,
}
