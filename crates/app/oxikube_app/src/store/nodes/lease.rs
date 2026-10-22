//! [`NodeLoadsLease`]: holds the cluster-wide Pod feed open and the pods-by-node index on.

use std::sync::Arc;

use oxikube_domain::Demand;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;

use super::PodsByNode;
use crate::store::object::{FeedKey, FeedScope, StoreObject};
use crate::store::policy::FeedKind;
use crate::store::query::{StoreFilter, StoreQuery};
use crate::store::service::ResourceStore;
use crate::store::subscription::Subscription;

/// The Pod kind the index is built from.
fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn pod_feed() -> FeedKey {
    FeedKey::new(pod_gvk(), FeedScope::Cluster)
}

/// A claim on the pods-by-node index of a [`ResourceStore`]. See the `store::nodes` module docs.
///
/// Taking one subscribes to the cluster-wide Pod feed through the normal path (the watch budget
/// decides, the feed is shared with the Pods table) and turns the index on; dropping it
/// releases the subscription and, with the last lease, turns the index off. Cheap to hold: the
/// subscription's filter matches nothing, so it keeps no row list.
pub struct NodeLoadsLease {
    store: ResourceStore,
    _subscription: Subscription,
}

impl std::fmt::Debug for NodeLoadsLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeLoadsLease").finish_non_exhaustive()
    }
}

impl ResourceStore {
    /// Opens (or joins) the cluster-wide Pod feed and builds the pods-by-node index on it for as
    /// long as the returned lease lives. Never blocks: the feed starts on the store's spawner, and
    /// so does the index's first build over pods the feed already holds (the lease answers `None`
    /// until it is done).
    pub fn lease_node_loads(&self) -> NodeLoadsLease {
        let query =
            StoreQuery::new(pod_gvk(), WatchScope::Cluster).with_filter(StoreFilter::nothing());
        let subscription = self.subscribe(query);
        let snapshot = self
            .inner()
            .with_entry_mut(&pod_feed(), |st| st.cache.track_nodes())
            .flatten();
        if let Some(snapshot) = snapshot {
            let generation = snapshot.generation;
            let store = self.clone();
            // The build belongs to the cache, not to this lease: another lease may be waiting on
            // it, and it is aborted when the last lease leaves.
            let build = self.inner().spawn(async move {
                let index = PodsByNode::build(snapshot.objects.iter());
                drop(snapshot.objects);
                store.inner().with_entry_mut(&pod_feed(), |st| {
                    st.cache.finish_nodes(snapshot.generation, index);
                });
            });
            self.inner().with_entry_mut(&pod_feed(), |st| {
                st.cache.hold_build(generation, build);
            });
        }
        NodeLoadsLease {
            store: self.clone(),
            _subscription: subscription,
        }
    }
}

impl NodeLoadsLease {
    /// Whether the index is complete: the Pod feed has listed and is live.
    pub fn is_ready(&self) -> bool {
        self.read(|_| ()).is_some()
    }

    /// The summed demand of the pods on `node`; `None` until the index is complete. A node with
    /// no pods answers zero.
    pub fn demand(&self, node: &str) -> Option<Demand> {
        self.read(|index| index.demand(node))
    }

    /// The pods on `node`, by namespace and name; `None` until the index is complete.
    pub fn pods(&self, node: &str) -> Option<Vec<Arc<StoreObject>>> {
        self.read(|index| index.pods(node))
    }

    /// [`demand`](Self::demand) and [`pods`](Self::pods) of `node` from one read, so the two agree.
    pub fn load(&self, node: &str) -> Option<(Demand, Vec<Arc<StoreObject>>)> {
        self.read(|index| (index.demand(node), index.pods(node)))
    }

    /// A number that changes whenever [`demand`](Self::demand) of some node may have: `0` while
    /// the index is not complete, then one that grows with every pod that arrives, leaves, moves
    /// or asks for other amounts (not with a status change). A table sorted by a node's load
    /// ranks again when it moves.
    pub fn epoch(&self) -> u64 {
        self.read(|index| index.version() + 1).unwrap_or(0)
    }

    fn read<R>(&self, f: impl FnOnce(&PodsByNode) -> R) -> Option<R> {
        self.store.inner().with_entry(&pod_feed(), |st| {
            // Only a full-object feed places pods: a feed the budget degraded to metadata (or
            // to a server-side table) holds partial resources, so its index would read as ready
            // and empty, which is wrong rather than unknown.
            if !st.feed_state.is_ready() || st.request.kind != FeedKind::Full {
                return None;
            }
            st.cache.pods_by_node().map(f)
        })?
    }
}

impl Drop for NodeLoadsLease {
    fn drop(&mut self) {
        self.store.inner().with_entry_mut(&pod_feed(), |st| {
            st.cache.untrack_nodes();
        });
    }
}
