//! [`AgentHooks`]: what hosted agents read of the app (E08-S09): the registry of `@`-mention
//! context providers, the registry of tools, and the queue "Send to agent" fills until the agent
//! panel (E27) takes it. Features register into the first two as they land; the MCP server and
//! the agent panel read them.

use std::sync::Arc;

use oxikube_app::context::{ContextRegistry, PendingContext};
use oxikube_app::tools::ToolRegistry;

/// The agent-facing registries of the app. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct AgentHooks {
    /// `@logs` and the other mention providers.
    pub contexts: Arc<ContextRegistry>,
    /// `k8s.get_logs` and the other tools.
    pub tools: Arc<ToolRegistry>,
    /// Context the user sent to the agent, waiting for the agent panel.
    pub pending: PendingContext,
}
