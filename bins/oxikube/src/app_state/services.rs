//! [`ClusterServices`]: the `oxikube_app` services of the cluster side, built over the ports.

use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{
    AliasRegistry, ClusterCatalog, ClusterCommands, ClusterSessionManager, IntegrationRegistry,
    MetricsService, MutationGuard, UtilisationAggregator,
};

use super::AppPorts;
use crate::metrics_provider::{self, MetricsServices};

/// The cluster services every window and the command bus share (E07-S00). Cheap to clone: each
/// is a handle on shared state.
///
/// Built with the [`AppState`](super::AppState), before any window: constructing them spawns
/// nothing and reads nothing. The `MutationGuard` is built with the command bus, which owns it
/// ([`ClusterServices::guard`]).
#[derive(Clone)]
pub struct ClusterServices {
    /// One session per opened cluster: connect, disconnect, state, the per-connection ports.
    pub sessions: ClusterSessionManager,
    /// The kubeconfig contexts with the user's favourites and last-used times.
    pub catalog: ClusterCatalog,
    /// The handler of `cluster::Connect`, `Reconnect`, `CancelConnect`, `Disconnect` and
    /// `ToggleFavourite`.
    pub cluster_commands: ClusterCommands,
    /// Namespace selection, favourites and the namespace list per cluster.
    pub namespaces: NamespaceService,
    /// Optional integrations (Argo CD later) and their sidebar sections.
    pub integrations: IntegrationRegistry,
    /// The `:` jump bar's words, one table per cluster (E11-S04): built-in k9s aliases, what each
    /// cluster's discovery serves, the user's `aliases.json`. Kept in step by
    /// [`crate::aliases`]; the jump bar reads `aliases.table(&cluster).resolve(word)`.
    pub aliases: AliasRegistry,
    /// Requests, limits and `%CPU/R`-style percents per pod, node and namespace (E13-S02). Its GPU
    /// vendor list follows `metrics.gpu_vendors` ([`crate::metrics_settings`]).
    pub utilisation: UtilisationAggregator,
    /// Which Prometheus-compatible service each cluster's history graphs come from, and the
    /// PromQL port that queries it (E13-S05). Idle until `metrics_provider::start`.
    pub metrics: MetricsServices,
    /// Live CPU and memory from metrics-server, polled per cluster while a table shows it
    /// (E13-S03). Idle until `live_metrics::start`.
    pub live_metrics: MetricsService,
}

impl ClusterServices {
    /// The services over `ports`.
    pub fn new(ports: &AppPorts) -> Self {
        let clusters = &ports.clusters;
        let sessions = ClusterSessionManager::new(
            clusters.connector.clone(),
            clusters.source.clone(),
            clusters.clock.clone(),
        );
        let catalog = ClusterCatalog::new(
            clusters.source.clone(),
            ports.state.clone(),
            clusters.clock.clone(),
        );
        let namespaces = NamespaceService::new(
            sessions.clone(),
            ports.state.clone(),
            clusters.clock.clone(),
        );
        let metrics = metrics_provider::build(ports, &sessions);
        let live_metrics = MetricsService::new(sessions.clone(), clusters.clock.clone());
        Self {
            cluster_commands: ClusterCommands::new(sessions.clone(), catalog.clone()),
            sessions,
            catalog,
            namespaces,
            integrations: IntegrationRegistry::new(),
            aliases: AliasRegistry::new(),
            utilisation: UtilisationAggregator::default(),
            metrics,
            live_metrics,
        }
    }

    /// A new `MutationGuard` over these sessions, auditing to `ports`' state db: the one the
    /// command bus is built with.
    pub fn guard(&self, ports: &AppPorts) -> MutationGuard {
        MutationGuard::new(
            self.sessions.clone(),
            ports.state.clone(),
            ports.clusters.clock.clone(),
        )
    }
}
