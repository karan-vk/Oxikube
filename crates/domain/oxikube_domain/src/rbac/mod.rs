//! RBAC policy as data (E17-S01): Roles, ClusterRoles and their bindings read from
//! [`Resource`](crate::Resource) JSON (no `k8s-openapi`), and the rule normaliser.
//!
//! [`access`](crate::access) answers "may I?" from a rules review; this module answers "what
//! may this subject do, and through which binding?" from the policy objects themselves. Both
//! share one matcher: a normalised resource rule *is* an [`AccessRule`](crate::access::AccessRule).
//!
//! | File | Holds |
//! |---|---|
//! | `kind` | [`RbacKind`] (the four kinds and their [`Gvk`](crate::ids::Gvk)s), [`RbacError`] |
//! | `subject` | [`Subject`] (User, Group, ServiceAccount) and [`UserInfo`]: who a request is made as, with the groups the API server adds |
//! | `policy` | [`PolicyRule`]: one rule as written |
//! | `selector` | [`AggregationRule`], [`RoleSelector`]: what an aggregated ClusterRole collects |
//! | `role` | [`Role`], [`RoleRef`], [`RoleId`], [`aggregated_rules`], [`aggregated_from`] |
//! | `binding` | [`Binding`], [`BindingRef`], [`GrantScope`] (cluster-wide or one namespace) |
//! | `normalise` | [`NormalisedRules`], [`NormalisedRule`], [`NonResourceRule`]: the normaliser |
//! | `matrix` | [`PolicyMatrix`]: k9s's per-group, per-verb table of a rule set |
//!
//! # Scope of a grant
//!
//! A ClusterRoleBinding grants cluster-wide. A RoleBinding grants inside its own namespace,
//! whether it names a Role (looked up in that namespace) or a ClusterRole. Non-resource URLs are
//! never namespaced, so only a ClusterRoleBinding grants them.
//!
//! # Subjects
//!
//! A ServiceAccount authenticates as the user `system:serviceaccount:<ns>:<name>` in the groups
//! `system:serviceaccounts` and `system:serviceaccounts:<ns>`; every authenticated subject is
//! also in `system:authenticated` ([`Subject::user_info`]). Those implicit groups are subjects of
//! their own only where a binding names them.

mod binding;
mod kind;
mod matrix;
mod normalise;
mod policy;
mod role;
mod selector;
mod subject;

#[cfg(test)]
mod tests;

pub use binding::{Binding, BindingKind, BindingRef, GrantScope};
pub use kind::{RBAC_GROUP, RbacError, RbacKind};
pub use matrix::{EXTRAS_HEADER, MATRIX_VERBS, MatrixRow, PolicyMatrix, verb_header};
pub use normalise::{NonResourceRule, NormalisedRule, NormalisedRules};
pub use policy::PolicyRule;
pub use role::{Role, RoleId, RoleKind, RoleRef, aggregated_from, aggregated_rules};
pub use selector::{AggregationRule, RoleSelector, SelectorRequirement};
pub use subject::{
    ANONYMOUS_USER, AUTHENTICATED_GROUP, SERVICE_ACCOUNTS_GROUP, Subject, UNAUTHENTICATED_GROUP,
    UserInfo,
};
