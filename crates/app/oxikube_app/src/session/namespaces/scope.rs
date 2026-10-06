//! [`ScopeDelta`]: what changes between two [`WatchScope`]s, so a store re-scopes its feeds
//! without reloading the namespaces that stay selected.

use std::collections::BTreeSet;

use oxikube_domain::session::WatchScope;

/// What happens to the cluster-wide feed of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterWide {
    /// It is neither started nor stopped.
    Unchanged,
    /// The scope became cluster-wide (`All`): start one feed, stop the per-namespace ones.
    Started,
    /// The scope narrowed to namespaces: stop the cluster-wide feed.
    Stopped,
}

/// The difference between an old and a new [`WatchScope`] of one kind.
///
/// `{a, b}` to `{b, c}` starts `c`, stops `a` and keeps `b`: the feed of `b` is untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDelta {
    /// Namespaces that need a new namespaced feed (sorted).
    pub started: Vec<String>,
    /// Namespaces whose feed is no longer needed (sorted).
    pub stopped: Vec<String>,
    /// Namespaces whose feed stays as it is (sorted).
    pub kept: Vec<String>,
    /// What happens to the cluster-wide feed.
    pub cluster_wide: ClusterWide,
}

impl ScopeDelta {
    /// The delta from `old` to `new`.
    pub fn between(old: &WatchScope, new: &WatchScope) -> Self {
        let old_set: BTreeSet<&str> = old.namespaces().iter().map(String::as_str).collect();
        let new_set: BTreeSet<&str> = new.namespaces().iter().map(String::as_str).collect();
        let own = |names: BTreeSet<&str>| names.into_iter().map(str::to_owned).collect();
        let cluster_wide = match (old, new) {
            (WatchScope::Cluster, WatchScope::Namespaces(_)) => ClusterWide::Stopped,
            (WatchScope::Namespaces(_), WatchScope::Cluster) => ClusterWide::Started,
            _ => ClusterWide::Unchanged,
        };
        Self {
            started: own(new_set.difference(&old_set).copied().collect()),
            stopped: own(old_set.difference(&new_set).copied().collect()),
            kept: own(new_set.intersection(&old_set).copied().collect()),
            cluster_wide,
        }
    }

    /// Whether nothing starts or stops.
    pub fn is_empty(&self) -> bool {
        self.started.is_empty()
            && self.stopped.is_empty()
            && self.cluster_wide == ClusterWide::Unchanged
    }
}
