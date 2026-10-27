//! What `k8s.can_i` and `@rbac` need, and registering them.

use std::sync::Arc;

use super::{CanITool, RbacContextProvider};
use crate::context::{ContextRegistry, RegisterProviderError};
use crate::tools::{RegisterToolError, ToolRegistry};

use super::CanIService;
use crate::session::ClusterSessionManager;

/// The services behind the can-i tool and `@rbac`. Cheap to clone.
#[derive(Clone)]
pub struct CanIDeps {
    /// The sessions: which cluster a call is about.
    pub sessions: ClusterSessionManager,
    /// Asks the cluster.
    pub service: CanIService,
}

impl std::fmt::Debug for CanIDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanIDeps").finish_non_exhaustive()
    }
}

/// Why [`install`] stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// The tool was refused.
    #[error(transparent)]
    Tool(#[from] RegisterToolError),
    /// The `@rbac` provider was refused.
    #[error(transparent)]
    Provider(#[from] RegisterProviderError),
}

/// Registers `k8s.can_i` on `tools` and `@rbac` on `contexts`.
///
/// # Errors
///
/// [`InstallError`] when a name is taken already; what was registered before it stays.
pub fn install(
    tools: &ToolRegistry,
    contexts: &ContextRegistry,
    deps: &CanIDeps,
) -> Result<(), InstallError> {
    tools.register(Arc::new(CanITool::new(deps.service.clone())))?;
    contexts.register(Arc::new(RbacContextProvider::new(deps.sessions.clone())))?;
    Ok(())
}
