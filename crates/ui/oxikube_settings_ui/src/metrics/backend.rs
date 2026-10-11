//! [`MetricsBackend`]: where the page reads its status from.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_app::{ClusterSessionManager, MetricsProviderService, ProviderStatus};
use oxikube_domain::ids::ClusterId;

/// What the page needs to know about a cluster right now.
#[derive(Clone, Debug)]
pub struct PageSource {
    /// The name to show for the cluster.
    pub title: String,
    /// Whether the cluster's session is connected.
    pub connected: bool,
    /// The provider status.
    pub status: Arc<ProviderStatus>,
}

/// The page's view of the app: reads are synchronous and in memory (the service keeps the
/// status), and changes arrive as a stream. Commands do not go through here: the page sends them
/// on the bus like every other view.
pub trait MetricsBackend: 'static {
    /// The cluster's current page source.
    fn page(&self, cluster: &ClusterId) -> PageSource;

    /// One item whenever [`page`](Self::page) may have a new answer for `cluster`.
    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()>;
}

/// The backend over the app's services.
#[derive(Clone)]
pub struct ServiceBackend {
    service: MetricsProviderService,
    sessions: ClusterSessionManager,
}

impl ServiceBackend {
    /// A backend over `service`, reading names and connection state from `sessions`.
    pub fn new(service: MetricsProviderService, sessions: ClusterSessionManager) -> Self {
        Self { service, sessions }
    }
}

impl MetricsBackend for ServiceBackend {
    fn page(&self, cluster: &ClusterId) -> PageSource {
        let session = self.sessions.get(cluster);
        PageSource {
            title: session
                .as_ref()
                .map_or_else(|| cluster.to_string(), |s| s.title().to_owned()),
            connected: session.as_ref().is_some_and(|s| s.is_connected()),
            status: self.service.status(cluster),
        }
    }

    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        // The service wakes for its own status; a session connecting or dropping changes what
        // the buttons may do, and the service answers that with a status change of its own.
        self.service.changes(cluster).boxed()
    }
}
