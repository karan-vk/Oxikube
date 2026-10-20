//! [`ForwardBackend`]: what the manager needs from the clusters (the port-forward port, the
//! read-only flag, watches of the target), and [`SessionBackend`], the one over the session
//! manager and the resource stores.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::PortForwardPort;

use crate::session::ClusterSessionManager;
use crate::store::{FeedState, ResourceStores, RowChange, StoreDelta, StoreObject, StoreQuery};

/// The objects matching a query, whole, each time they change: `Err` when the feed stopped for
/// good (forbidden, unknown kind). The first item comes once the feed has listed.
pub type ObjectWatch = BoxStream<'static, Result<Vec<Resource>, String>>;

/// The manager's view of the clusters. [`SessionBackend`] is the app's; tests fake it.
pub trait ForwardBackend: Send + Sync {
    /// The connected cluster's port-forward port; `None` while it is not connected.
    fn port_forward(&self, cluster: &ClusterId) -> Option<Arc<dyn PortForwardPort>>;

    /// Whether the cluster is in read-only mode.
    fn is_read_only(&self, cluster: &ClusterId) -> bool;

    /// A watch of `query` in `cluster` (ref-counted feeds, no polling); `None` while the cluster
    /// is not connected.
    fn watch(&self, cluster: &ClusterId, query: StoreQuery) -> Option<ObjectWatch>;
}

/// [`ForwardBackend`] over the sessions and the shared [`ResourceStores`]: a forward's watch is
/// one more subscriber of the feeds the tables already hold.
#[derive(Clone)]
pub struct SessionBackend {
    sessions: ClusterSessionManager,
    stores: Arc<ResourceStores>,
}

impl SessionBackend {
    /// A backend over `sessions` and `stores`.
    pub fn new(sessions: ClusterSessionManager, stores: Arc<ResourceStores>) -> Self {
        Self { sessions, stores }
    }
}

impl ForwardBackend for SessionBackend {
    fn port_forward(&self, cluster: &ClusterId) -> Option<Arc<dyn PortForwardPort>> {
        self.sessions.get(cluster)?.port_forward()
    }

    fn is_read_only(&self, cluster: &ClusterId) -> bool {
        self.sessions.is_read_only(cluster)
    }

    fn watch(&self, cluster: &ClusterId, query: StoreQuery) -> Option<ObjectWatch> {
        let session = self.sessions.get(cluster)?;
        let subscription = self.stores.for_session(&session)?.subscribe(query);
        let mut rows: Vec<Arc<StoreObject>> = Vec::new();
        let mut emitted = false;
        Some(
            subscription
                .filter_map(move |delta| {
                    futures::future::ready(watch_item(&delta, &mut rows, &mut emitted))
                })
                .boxed(),
        )
    }
}

/// What one store delta means to a forward's watch: `None` when it has nothing new.
///
/// A name-filtered subscription also wakes when the namespace's object count moves (any pod
/// added or deleted there) with its rows `Unchanged`; once a snapshot went out, such a wake is
/// skipped, so a forward re-resolves on changes to its target only, not on namespace churn.
pub(super) fn watch_item(
    delta: &StoreDelta,
    rows: &mut Vec<Arc<StoreObject>>,
    emitted: &mut bool,
) -> Option<Result<Vec<Resource>, String>> {
    delta.apply_to(rows);
    if delta.state.is_terminal() {
        return Some(Err(feed_failure(&delta.state)));
    }
    if !delta.state.is_ready() || (*emitted && matches!(delta.rows, RowChange::Unchanged)) {
        return None;
    }
    *emitted = true;
    Some(Ok(rows
        .iter()
        .filter_map(|o| o.resource().cloned())
        .collect()))
}

/// One line on why a feed stopped.
fn feed_failure(state: &FeedState) -> String {
    match state {
        FeedState::Forbidden { message } => format!("not allowed to watch the target: {message}"),
        FeedState::Unauthorized { message } => format!("the credentials were rejected: {message}"),
        FeedState::Failed { message, .. } => format!("the target cannot be watched: {message}"),
        FeedState::Warming | FeedState::Ready | FeedState::Retrying { .. } => String::new(),
    }
}
