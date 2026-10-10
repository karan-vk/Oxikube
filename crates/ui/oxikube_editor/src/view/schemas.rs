//! Where a manifest editor gets its schemas: the `SchemaPort` (E10-S01) of its cluster's
//! session, the only schema source. Asked per fetch, so an editor opened while the cluster was
//! connected does not keep a disconnected session's ports alive.

use std::sync::Arc;

use gpui::{App, SharedString};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::SchemaPort;

/// The schemas of one cluster.
pub trait SchemaSource: 'static {
    /// The cluster the schemas are for.
    fn cluster(&self) -> &ClusterId;

    /// The cluster's name as the editor shows it ("kind-oxikube").
    fn label(&self, cx: &App) -> SharedString;

    /// The port to ask, `None` while the cluster is not connected (the kinds are asked for
    /// again on the next validation).
    fn port(&self, cx: &App) -> Option<Arc<dyn SchemaPort>>;
}

/// The app's [`SchemaSource`]: the session of `cluster`.
#[derive(Clone)]
pub struct SessionSchemas {
    sessions: ClusterSessionManager,
    cluster: ClusterId,
}

impl SessionSchemas {
    /// The schemas of `cluster`'s session.
    pub fn new(sessions: ClusterSessionManager, cluster: ClusterId) -> Self {
        Self { sessions, cluster }
    }
}

impl SchemaSource for SessionSchemas {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn label(&self, _: &App) -> SharedString {
        match self.sessions.get(&self.cluster) {
            Some(session) => SharedString::from(session.title().to_owned()),
            None => SharedString::from(self.cluster.to_string()),
        }
    }

    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        self.sessions.get(&self.cluster)?.schemas()
    }
}
