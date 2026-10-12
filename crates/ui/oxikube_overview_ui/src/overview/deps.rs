//! [`OverviewDeps`]: what a cluster overview is built from.

use std::rc::Rc;
use std::sync::Arc;

use oxikube_app::cluster_overview::ProviderSource;
use oxikube_app::{ClusterSessionManager, MetricsService, RecentWarnings, ResourceStores};
use oxikube_ports::{PromqlPort, QueryCataloguePort};
use oxikube_workspace::CommandDispatcher;

/// What the overview reads, and where its clicks go.
#[derive(Clone)]
pub struct OverviewDeps {
    /// The sessions: the cluster's connection, context and server, and their updates.
    pub sessions: ClusterSessionManager,
    /// The per-cluster stores the overview's Pod and Node feeds come from.
    pub stores: Arc<ResourceStores>,
    /// Where clicks send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Live CPU and memory (metrics-server); `None` draws `metrics unavailable` in their place.
    pub metrics: Option<MetricsService>,
    /// History graphs (Prometheus); `None` draws no graphs.
    pub history: Option<HistoryDeps>,
    /// The recent warnings (the Events feed, E14, when it exists).
    pub warnings: Arc<dyn RecentWarnings>,
}

/// What the history graphs need: whether a cluster has a Prometheus-compatible endpoint, the
/// queries for its stack, and the port that runs them.
#[derive(Clone)]
pub struct HistoryDeps {
    /// Availability per cluster (the app's `MetricsProviderService`).
    pub source: Arc<dyn ProviderSource>,
    /// The per-cluster PromQL port.
    pub promql: Arc<dyn PromqlPort>,
    /// The query catalogue (E13-S06).
    pub catalogue: Arc<dyn QueryCataloguePort>,
}
