//! Subject-centric RBAC (E17-S07): who the cluster's bindings name, and what each user, group or
//! ServiceAccount may do, as k9s's per-verb policy matrix with the bindings and roles that grant
//! every cell.
//!
//! The data is the *policy* (Roles, ClusterRoles and bindings), not a review of one identity, so
//! it answers "what does the policy grant this subject?" for any subject, whoever is signed in.
//! It is built on the E17-S01 [`SubjectIndex`](crate::access::SubjectIndex) and normaliser; there
//! is no second matcher.
//!
//! | File | Holds |
//! |---|---|
//! | `feed` | [`PolicyFeed`], [`PolicyHandle`], [`PolicyStatus`]: the four store feeds applied to the index, and the shared view of it |
//! | `summary` | [`SubjectList`], [`SubjectSummary`]: the rows of the Users and Groups pages |
//! | `matrix` | [`SubjectMatrix`], [`MatrixLine`], [`MatrixColumn`], [`Provenance`], [`Inherit`], the tab-separated export |
//! | `filter` | [`SubjectFilter`]: the `/` filter grammar over rows |
//! | `service` | [`SubjectsService`]: opens the feeds on a cluster's store, remembers the subject names `:rbac` completes |
//!
//! # Cost
//!
//! The Roles and bindings arrive through the store's shared feeds (full objects, one list and one
//! watch per kind, cluster-wide), so a page open next to the Roles and Bindings tables costs no
//! extra request. The Users and Groups pages read the bindings only. A subject's matrix is
//! computed when it is selected: the roles it is bound to are normalised once (the index caches
//! them), merged and projected, on a background task, never on the UI thread.
//!
//! # What the API cannot say
//!
//! Kubernetes does not list which users belong to a group, so a group's page shows what the
//! bindings grant to anyone in it and no members ([`GROUP_MEMBERS_NOTE`]).

mod cells;
mod feed;
mod filter;
mod matrix;
mod service;
mod summary;

#[cfg(test)]
mod tests;

pub use feed::{PolicyFeed, PolicyHandle, PolicyStatus};
pub use filter::SubjectFilter;
pub use matrix::{
    Inherit, LineKind, MatrixColumn, MatrixLine, Provenance, SubjectMatrix, provenance, scopes_of,
    subject_matrix,
};
pub use service::SubjectsService;
pub use summary::{SubjectList, SubjectSummary, resolve_subject, subject_name, summaries};

/// What the Groups page says about membership.
pub const GROUP_MEMBERS_NOTE: &str = "Kubernetes does not list the members of a group: these \
rows show what the bindings grant to anyone the authenticator puts in the group.";
