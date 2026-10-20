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
use crate::store::{FeedState, ResourceStores, StoreObject, StoreQuery};

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
        Some(
            subscription
                .filter_map(move |delta| {
                    delta.apply_to(&mut rows);
                    let item = if delta.state.is_terminal() {
                        Some(Err(feed_failure(&delta.state)))
                    } else if delta.state.is_ready() {
                        Some(Ok(rows
                            .iter()
                            .filter_map(|o| o.resource().cloned())
                            .collect()))
                    } else {
                        None
                    };
                    futures::future::ready(item)
                })
                .boxed(),
        )
    }
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
