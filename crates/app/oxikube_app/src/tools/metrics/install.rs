//! What the metrics tools and `@metrics` need, and registering them.

use std::sync::Arc;

use oxikube_ports::{ClockPort, PromqlPort, QueryCataloguePort};

use super::{QueryPromqlTool, TopNodesTool, TopPodsTool};
use crate::context::{ContextRegistry, MetricsContextProvider, RegisterProviderError};
use crate::metrics::live::MetricsService;
use crate::metrics_provider::MetricsProviderService;
use crate::session::ClusterSessionManager;
use crate::tools::{RegisterToolError, ToolRegistry};

/// The services behind the metrics tools and the `@metrics` mention. Cheap to clone.
#[derive(Clone)]
pub struct MetricsAgentDeps {
    /// The sessions: which cluster a call is about, and its objects.
    pub sessions: ClusterSessionManager,
    /// Live usage from metrics-server: on-demand reads, and the ring buffers `@metrics` quotes.
    pub live: MetricsService,
    /// Which Prometheus-compatible stack each cluster has, and whether its settings allow it.
    pub provider: MetricsProviderService,
    /// PromQL access per cluster.
    pub promql: Arc<dyn PromqlPort>,
    /// The PromQL of a catalogue request for a stack.
    pub catalogue: Arc<dyn QueryCataloguePort>,
    /// Deadlines.
    pub clock: Arc<dyn ClockPort>,
}

impl std::fmt::Debug for MetricsAgentDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsAgentDeps").finish_non_exhaustive()
    }
}

/// Why [`install`] stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// A tool was refused.
    #[error(transparent)]
    Tool(#[from] RegisterToolError),
    /// The `@metrics` provider was refused.
    #[error(transparent)]
    Provider(#[from] RegisterProviderError),
}

/// Registers `k8s.top_pods`, `k8s.top_nodes` and `k8s.query_promql` on `tools` and the
/// `@metrics` provider on `contexts`, all reading through `deps`.
///
/// # Errors
///
/// [`InstallError`] when one of the names is taken already; what was registered before it stays.
pub fn install(
    tools: &ToolRegistry,
    contexts: &ContextRegistry,
    deps: &MetricsAgentDeps,
) -> Result<(), InstallError> {
    tools.register(Arc::new(TopPodsTool::new(deps.clone())))?;
    tools.register(Arc::new(TopNodesTool::new(deps.clone())))?;
    tools.register(Arc::new(QueryPromqlTool::new(deps.clone())))?;
    contexts.register(Arc::new(MetricsContextProvider::new(deps.clone())))?;
    Ok(())
}
