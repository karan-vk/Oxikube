//! [`ReflectorFeed`]: the consumer's handle on a running feed.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use kube::runtime::reflector::Store;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{DeltaBatch, WatchFeed};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::object::FeedObject;
use super::state::FeedState;

/// What a feed watches. Together with the cluster of the [`KubeResources`](crate::KubeResources)
/// that opened it (one per cluster session), this is the feed key (cluster, gvk, scope).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedKey {
    /// The watched kind.
    pub gvk: Gvk,
    /// The whole cluster, or the namespaces watched.
    pub scope: WatchScope,
}

/// A live reflector feed: a stream of coalesced [`DeltaBatch`]es over a bounded channel,
/// its [`FeedState`], and read access to its reflector store.
///
/// The first batch starts with a `Restarted` holding the complete list; later batches carry
/// `Applied` / `Deleted` deltas (a relist arrives as its diff unless
/// [`RelistDelivery::Snapshot`](super::RelistDelivery::Snapshot) is set). Retryable failures
/// arrive as `Err` items while the feed backs off and continues; a non-retryable one is the
/// last item.
///
/// Dropping the handle aborts the feed task and with it every watch request (the same
/// abort-on-drop contract as `oxikube_runtime::spawn_kube`).
pub struct ReflectorFeed {
    pub(super) key: FeedKey,
    pub(super) batches: mpsc::Receiver<OxiResult<DeltaBatch<Resource>>>,
    pub(super) state: watch::Receiver<FeedState>,
    pub(super) stores: Vec<Store<FeedObject>>,
    pub(super) metadata_only: bool,
    pub(super) task: JoinHandle<()>,
}

impl ReflectorFeed {
    /// What this feed watches.
    pub fn key(&self) -> &FeedKey {
        &self.key
    }

    /// Whether this is a metadata-only feed: every resource it delivers is
    /// [partial](Resource::is_partial).
    pub fn is_metadata_only(&self) -> bool {
        self.metadata_only
    }

    /// The feed's state, updated as it changes.
    pub fn state(&self) -> watch::Receiver<FeedState> {
        self.state.clone()
    }

    /// The reflector store's current contents (all namespaces of the scope), shared, not
    /// copied. Empty until the first list completes.
    pub fn snapshot(&self) -> Vec<Arc<FeedObject>> {
        self.stores.iter().flat_map(Store::state).collect()
    }

    /// Whether the feed task has ended (non-retryable error, or aborted).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// The feed as the port's [`WatchFeed`].
    pub fn into_watch_feed(self) -> WatchFeed<Resource> {
        Box::pin(self)
    }
}

impl Stream for ReflectorFeed {
    type Item = OxiResult<DeltaBatch<Resource>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.batches.poll_recv(cx)
    }
}

impl Drop for ReflectorFeed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::fmt::Debug for ReflectorFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflectorFeed")
            .field("key", &self.key)
            .field("state", &*self.state.borrow())
            .field("metadata_only", &self.metadata_only)
            .finish_non_exhaustive()
    }
}
