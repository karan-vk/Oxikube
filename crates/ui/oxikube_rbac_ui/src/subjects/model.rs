//! The page as plain data: which page it is, how far loading is.

use oxikube_domain::rbac::GrantScope;

/// What the page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The users and ServiceAccounts the bindings name (`:usr`).
    Users,
    /// The groups the bindings name (`:grp`).
    Groups,
    /// One subject's matrix (`:rbac <subject>`).
    Subject,
}

/// Where loading stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The policy feeds have not all listed: how many are still to.
    Loading {
        /// Feeds pending, of four.
        pending: usize,
    },
    /// The feeds listed; the rows are current.
    Ready,
    /// There is no policy to show (the cluster is not connected).
    Unavailable(String),
}

/// A scope as the chips read: `Cluster-wide`, `ns/web`.
pub fn scope_label(scope: &GrantScope) -> String {
    match scope {
        GrantScope::Cluster => "Cluster-wide".to_owned(),
        GrantScope::Namespace(ns) => format!("Namespace {ns}"),
    }
}
