//! Reading the caches without subscribing (E12-S16): a count of one namespace's objects and the
//! objects of a cluster-scoped kind, for the Namespaces table's pod counts and the delete dialog's
//! warnings.
//!
//! Like [`ResourceStore::counts`], these only read: a kind nobody watches answers `None` and no
//! feed is started. A read takes the store's entry map lock and one entry's lock for as long as
//! it takes to look something up; nothing is cloned but `Arc`s.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;

use super::delta::FeedState;
use super::entry::EntryState;
use super::object::{FeedKey, FeedScope, StoreObject};
use super::service::ResourceStore;

/// Whether the rows of a feed in `state` can be trusted: listed (or kept while it retries).
fn listed(state: &EntryState) -> bool {
    matches!(
        state.feed_state,
        FeedState::Ready | FeedState::Retrying { .. }
    )
}

impl ResourceStore {
    /// How many objects of `gvk` are in `namespace`, read from a feed that is already open and
    /// listed: the kind's cluster-wide feed, else the namespace's own. `None` when no open
    /// feed covers the namespace (nothing watches the kind, the list has not arrived, or the
    /// feed was refused), so a caller shows "unknown" instead of a zero that may be wrong.
    ///
    /// O(1): the cache keeps an index by namespace, updated as events apply.
    pub fn count_in_namespace(&self, gvk: &Gvk, namespace: &str) -> Option<usize> {
        let counted = |key: &FeedKey| {
            self.inner()
                .with_entry(key, |st| {
                    listed(st).then(|| st.cache.count_in_namespace(namespace))
                })
                .flatten()
        };
        // The cluster-wide feed first: it knows every namespace, and its key needs no allocation
        // (a namespace's key holds an `Arc<str>` of the name), which matters for a column read
        // per visible row.
        counted(&FeedKey::new(gvk.clone(), FeedScope::Cluster)).or_else(|| {
            counted(&FeedKey::new(
                gvk.clone(),
                FeedScope::Namespace(namespace.into()),
            ))
        })
    }

    /// The cached cluster-scoped object `name` of `gvk`, from the kind's cluster-wide feed.
    pub fn cluster_object(&self, gvk: &Gvk, name: &str) -> Option<Arc<StoreObject>> {
        let key = FeedKey::new(gvk.clone(), FeedScope::Cluster);
        self.inner()
            .with_entry(&key, |st| st.cache.cluster_object(name).cloned())
            .flatten()
    }

    /// Every cached object of the cluster-wide feed of `gvk`, in no particular order. `None`
    /// while that feed is not listed. For kinds with few objects (namespaces, nodes): the clone
    /// is one `Arc` per object.
    pub fn cluster_objects(&self, gvk: &Gvk) -> Option<Vec<Arc<StoreObject>>> {
        let key = FeedKey::new(gvk.clone(), FeedScope::Cluster);
        self.inner()
            .with_entry(&key, |st| {
                listed(st).then(|| st.cache.values().cloned().collect())
            })
            .flatten()
    }
}
