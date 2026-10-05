//! [`SelectionLease`]: the feeds of one kind under a [`NamespaceSelection`], one per
//! namespace.
//!
//! `NamespaceSelection::All` (or a cluster-scoped kind) is one cluster-wide feed; a `Set` is
//! one namespaced feed per name (`Api::namespaced_with` under the reflector feed), so
//! narrowing the selection never relists what stays selected. [`SelectionLease::reselect`]
//! applies a new selection: it keeps the feeds of namespaces that stay, releases the others
//! (they idle for the grace period, so switching back is instant) and opens the new ones.

use std::collections::BTreeMap;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::Scope;
use oxikube_domain::session::{NamespaceSelection, WatchScope};
use tracing::debug;

use super::lease::FeedLease;
use super::policy::{ScopeChange, plan};
use super::registry::FeedRegistry;
use super::request::FeedRequest;
use super::source::FeedStream;

/// The per-namespace leases of one kind under a namespace selection.
pub struct SelectionLease {
    registry: FeedRegistry,
    /// The request every namespace's feed is made from (its namespace is ignored).
    template: FeedRequest,
    kind_scope: Scope,
    /// One lease per namespace; `None` is the cluster-wide feed.
    leases: BTreeMap<Option<String>, FeedLease>,
}

impl FeedRegistry {
    /// Subscribes to `template`'s kind under `selection`: one feed per selected namespace for
    /// a namespaced kind and a `Set`, one cluster-wide feed otherwise.
    ///
    /// # Errors
    ///
    /// As [`subscribe`](Self::subscribe), for the first feed that fails; no lease is kept
    /// then.
    pub async fn subscribe_selection(
        &self,
        template: FeedRequest,
        kind_scope: Scope,
        selection: &NamespaceSelection,
    ) -> OxiResult<SelectionLease> {
        let mut lease = SelectionLease {
            registry: self.clone(),
            template,
            kind_scope,
            leases: BTreeMap::new(),
        };
        lease.reselect(selection).await?;
        Ok(lease)
    }
}

impl SelectionLease {
    /// The watch scope the leases cover now.
    pub fn scope(&self) -> WatchScope {
        if self.leases.contains_key(&None) {
            return WatchScope::Cluster;
        }
        WatchScope::Namespaces(self.leases.keys().flatten().cloned().collect())
    }

    /// The lease of each namespace (`None`: the cluster-wide feed), in order.
    pub fn leases(&self) -> impl Iterator<Item = (Option<&str>, &FeedLease)> {
        self.leases.iter().map(|(ns, lease)| (ns.as_deref(), lease))
    }

    /// Moves out the streams of the feeds opened since the last call, by namespace.
    pub fn take_feeds(&mut self) -> Vec<(Option<String>, FeedStream)> {
        self.leases
            .iter_mut()
            .filter_map(|(ns, lease)| lease.take_feed().map(|feed| (ns.clone(), feed)))
            .collect()
    }

    /// Switches to `selection`: keeps the feeds of namespaces that stay selected, releases
    /// the others and opens (or rejoins) the new ones. Returns what changed.
    ///
    /// Leaving namespaces are released first, so their feeds are idle and the budget can
    /// tear them down to make room for the new ones.
    ///
    /// # Errors
    ///
    /// As [`FeedRegistry::subscribe`]. The selection is then rolled back: the new leases are
    /// dropped and the released namespaces subscribed again (a rejoin while their feeds idle).
    pub async fn reselect(&mut self, selection: &NamespaceSelection) -> OxiResult<ScopeChange> {
        let target = WatchScope::derive(selection, self.kind_scope);
        let change = plan(self.leases.keys(), &target);
        if change.is_empty() {
            return Ok(change);
        }
        for namespace in &change.stop {
            self.leases.remove(namespace);
        }
        let mut opened = Vec::with_capacity(change.start.len());
        for namespace in &change.start {
            match self.subscribe(namespace.clone()).await {
                Ok(lease) => opened.push((namespace.clone(), lease)),
                Err(err) => {
                    drop(opened);
                    self.restore(&change.stop).await;
                    return Err(err);
                }
            }
        }
        self.leases.extend(opened);
        debug!(
            kind = %self.template.gvk,
            started = change.start.len(),
            kept = change.keep.len(),
            stopped = change.stop.len(),
            "namespace selection applied",
        );
        Ok(change)
    }

    async fn subscribe(&self, namespace: Option<String>) -> OxiResult<FeedLease> {
        let request = self.template.clone().in_namespace(namespace);
        self.registry.subscribe(request).await
    }

    /// Best effort: subscribes `namespaces` again after a failed reselect.
    async fn restore(&mut self, namespaces: &[Option<String>]) {
        for namespace in namespaces {
            match self.subscribe(namespace.clone()).await {
                Ok(lease) => {
                    self.leases.insert(namespace.clone(), lease);
                }
                Err(err) => {
                    debug!(kind = %self.template.gvk, error_kind = err.kind().as_str(), "could not restore a namespace feed");
                }
            }
        }
    }
}

impl std::fmt::Debug for SelectionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionLease")
            .field("kind", &self.template.gvk)
            .field("scope", &self.scope())
            .finish_non_exhaustive()
    }
}
