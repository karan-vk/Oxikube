//! One-shot reads of the metrics API for callers that are not a table: the agent tools and the
//! `@metrics` mention (E13-S12).
//!
//! The pollers fill [`ClusterUsage`](super::ClusterUsage) only while a table is watching and its
//! tab is shown, so an agent asking `top_pods` between two such moments would find nothing. A
//! snapshot is the same call a poll makes, on demand, answered to the caller and **not** written
//! to the cache: a cache that only a poll writes keeps its epoch, its state and its rings honest
//! for the columns that read them.

use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{MetricsOutcome, MetricsPort};

use super::service::MetricsService;
use crate::session::ClusterSession;

impl MetricsService {
    /// One call for every node's usage in `cluster`, answered to the caller only. The session
    /// must be connected.
    ///
    /// # Errors
    ///
    /// `NotFound` for a cluster with no session, `Network` for one that is not connected, and
    /// whatever the metrics port returns for a failed call. An absent or denied metrics API is
    /// `Ok(MetricsOutcome::Unavailable(..))`, as for a poll.
    pub async fn node_snapshot(&self, cluster: &ClusterId) -> OxiResult<MetricsOutcome> {
        let port = self.port_of(cluster)?;
        port.node_metrics(cluster).await
    }

    /// One call for every pod's usage in `namespaces` of `cluster`, answered to the caller only.
    ///
    /// # Errors
    ///
    /// As [`node_snapshot`](Self::node_snapshot).
    pub async fn pod_snapshot(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<MetricsOutcome> {
        let port = self.port_of(cluster)?;
        port.pod_metrics(cluster, namespaces).await
    }

    fn port_of(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn MetricsPort>> {
        let session: ClusterSession = self
            .inner
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found(format!("no session for cluster {cluster}")))?;
        session.metrics().ok_or_else(|| {
            OxiError::network(format!("cluster {} is not connected", session.title()))
        })
    }
}
