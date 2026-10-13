//! What the events tools, `@events` and "Send to agent" need, and registering them.

use std::sync::Arc;

use oxikube_ports::ClockPort;

use super::GetEventsTool;
use super::provider::EventsContextProvider;
use crate::context::{ContextRegistry, RegisterProviderError};
use crate::events::EventService;
use crate::search::aliases::AliasRegistry;
use crate::session::ClusterSessionManager;
use crate::tools::{RegisterToolError, ToolRegistry};

/// The services behind `k8s.get_events`, `@events` and `events::SendToAgent`. Cheap to clone.
#[derive(Clone)]
pub struct EventsAgentDeps {
    /// The sessions: which cluster a call is about, and its objects.
    pub sessions: ClusterSessionManager,
    /// The events of every cluster: the one buffer every events consumer reads.
    pub events: EventService,
    /// The kind words of each cluster (`pod`, `deploy`, `Deployment`) for `@events` and the
    /// tool's `involved.kind`.
    pub aliases: AliasRegistry,
    /// Deadlines and the age of an event.
    pub clock: Arc<dyn ClockPort>,
}

impl std::fmt::Debug for EventsAgentDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsAgentDeps").finish_non_exhaustive()
    }
}

/// Why [`install`] stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// The tool was refused.
    #[error(transparent)]
    Tool(#[from] RegisterToolError),
    /// The `@events` provider was refused.
    #[error(transparent)]
    Provider(#[from] RegisterProviderError),
}

/// Registers `k8s.get_events` on `tools` and the `@events` provider on `contexts`, both reading
/// through `deps`.
///
/// # Errors
///
/// [`InstallError`] when one of the names is taken already; what was registered before it stays.
pub fn install(
    tools: &ToolRegistry,
    contexts: &ContextRegistry,
    deps: &EventsAgentDeps,
) -> Result<(), InstallError> {
    tools.register(Arc::new(GetEventsTool::new(deps.clone())))?;
    contexts.register(Arc::new(EventsContextProvider::new(deps.clone())))?;
    Ok(())
}
