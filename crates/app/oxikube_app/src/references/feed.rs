//! [`ReferenceFeed`]: the task that keeps a [`ReferenceIndex`] current from a cluster's Pods.

use std::sync::Arc;

use futures::{FutureExt as _, StreamExt as _, select_biased};
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::WatchScope;

use super::index::ReferenceIndex;
use super::workloads::OwnerCache;
use crate::session::ClusterSessionManager;
use crate::store::{ResourceStore, StoreQuery, TaskGuard};

/// A running [`ReferenceIndex`] over one store's Pods feed.
///
/// Starting it subscribes to the cluster's Pods through the store (a feed the Pods table and the
/// overview share, so usually already warm; the watch budget decides, as for any view). The task
/// follows the session's namespace selection: when it changes the Pods subscription is moved to
/// the new scope (the store re-sends a snapshot), so the index covers what the tables list.
/// Dropping the feed aborts the task (the store keeps the guard, never the task itself), which
/// releases the subscription and, after the store's grace period, the feed. A
/// [`ReferenceHub`](super::ReferenceHub) shares one feed between the views that need it, so the
/// subscription lives exactly as long as the last of them.
pub struct ReferenceFeed {
    index: Arc<ReferenceIndex>,
    owners: Arc<OwnerCache>,
    store: ResourceStore,
    _task: TaskGuard,
}

impl std::fmt::Debug for ReferenceFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReferenceFeed")
            .field("cluster", self.store.cluster())
            .finish_non_exhaustive()
    }
}

impl ReferenceFeed {
    /// Starts following the Pods of `store`'s cluster.
    pub fn start(store: &ResourceStore, sessions: &ClusterSessionManager) -> Self {
        let index = Arc::new(ReferenceIndex::new());
        let task = store.spawn(follow(
            store.clone(),
            sessions.clone(),
            store.cluster().clone(),
            index.clone(),
        ));
        Self {
            index,
            owners: Arc::new(OwnerCache::default()),
            store: store.clone(),
            _task: task,
        }
    }

    /// The index this feed keeps.
    pub fn index(&self) -> &Arc<ReferenceIndex> {
        &self.index
    }

    /// The owners already resolved to their workload, shared by every "used by" read of the feed.
    pub fn owners(&self) -> &Arc<OwnerCache> {
        &self.owners
    }

    /// Whether this feed reads `store` (a reconnect builds a new store, and the old feed's Pods
    /// are stale).
    pub fn reads(&self, store: &ResourceStore) -> bool {
        self.store.is_same(store)
    }
}

/// The scope the Pods are read in: the session's current namespace selection.
fn pods_scope(sessions: &ClusterSessionManager, cluster: &ClusterId) -> WatchScope {
    sessions
        .get(cluster)
        .map_or(WatchScope::Cluster, |session| {
            session.watch_scope(Scope::Namespaced)
        })
}

async fn follow(
    store: ResourceStore,
    sessions: ClusterSessionManager,
    cluster: ClusterId,
    index: Arc<ReferenceIndex>,
) {
    let gvk = Gvk::new("", "v1", "Pod");
    let scope = pods_scope(&sessions, &cluster);
    index.set_scope(scope.clone());
    let mut subscription = store.subscribe(StoreQuery::new(gvk, scope));
    let mut updates = sessions.subscribe();
    loop {
        select_biased! {
            delta = subscription.next().fuse() => match delta {
                Some(delta) => index.apply(&delta),
                None => break,
            },
            update = updates.next().fuse() => match update {
                // Another cluster's change is not ours; a lagged receiver may have missed ours.
                Some(Ok(update)) if update.cluster != cluster => {}
                Some(_) => {
                    let scope = pods_scope(&sessions, &cluster);
                    if scope != subscription.query().scope {
                        index.set_scope(scope.clone());
                        subscription.rescope(scope);
                    }
                }
                None => break,
            },
        }
    }
}
