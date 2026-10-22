//! [`ReleaseResourcesLease`]: the feeds a release's Resources tab needs, held while it is open.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::session::WatchScope;

use super::join::{ReleaseResource, join};
use super::plan::ResourcePlan;
use crate::store::{ResourceStore, StoreFilter, StoreQuery, Subscription};

/// A claim on the feeds that serve one release's objects. See the [module docs](super).
///
/// Taking one subscribes (ref-counted, through the normal budget) to each served kind the plan
/// uses: namespaced kinds per namespace the plan touches, cluster-scoped kinds cluster-wide.
/// Subscriptions use a filter nothing passes, so no row list is kept. Dropping the lease (the
/// tab closing) releases them; a feed another view still holds stays open.
pub struct ReleaseResourcesLease {
    store: ResourceStore,
    plan: Arc<ResourcePlan>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ReleaseResourcesLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseResourcesLease")
            .field("resources", &self.plan.resources.len())
            .finish_non_exhaustive()
    }
}

impl ReleaseResourcesLease {
    /// Subscribes to the feeds `plan` needs. Never blocks: feeds start on the store's spawner.
    pub fn new(store: &ResourceStore, plan: Arc<ResourcePlan>) -> Self {
        // One query per kind: a namespaced kind watches the namespaces the plan touches (empty
        // set: cluster-scoped, one cluster-wide feed).
        let mut by_kind: BTreeMap<Gvk, BTreeSet<String>> = BTreeMap::new();
        for planned in plan.resources.iter().filter(|p| p.scope.is_some()) {
            let r = &planned.reference;
            let namespaces = by_kind.entry(r.gvk.clone()).or_default();
            if let (Some(Scope::Namespaced), Some(ns)) = (planned.scope, r.namespace()) {
                namespaces.insert(ns.to_owned());
            }
        }
        let subscriptions = by_kind
            .into_iter()
            .map(|(gvk, namespaces)| {
                let scope = if namespaces.is_empty() {
                    WatchScope::Cluster
                } else {
                    WatchScope::Namespaces(namespaces.into_iter().collect())
                };
                store.subscribe(StoreQuery::new(gvk, scope).with_filter(StoreFilter::nothing()))
            })
            .collect();
        Self {
            store: store.clone(),
            plan,
            _subscriptions: subscriptions,
        }
    }

    /// The release's objects with their current live state. Cheap (see [`join`]): call it on
    /// each refresh; the store's feeds keep it current.
    pub fn resources(&self) -> Vec<ReleaseResource> {
        join(&self.plan, &self.store)
    }

    /// The plan this lease serves.
    pub fn plan(&self) -> &Arc<ResourcePlan> {
        &self.plan
    }
}
