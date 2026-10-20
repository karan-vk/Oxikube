//! Registering the port-forward tools.

use std::sync::Arc;

use super::{ForwardPermission, ListPortForwardsTool, StartPortForwardTool};
use crate::portforward::PortForwardManager;
use crate::tools::{RegisterToolError, ToolRegistry};

/// Registers `portforward.list_port_forwards` and the gated `portforward.start_port_forward` on
/// `tools`, both over `manager`; `permission` decides who may start a forward (pass
/// [`DenyForwards`](super::DenyForwards) until the agent permission prompt exists).
///
/// # Errors
///
/// [`RegisterToolError`] when one of the names is taken; what was registered before stays.
pub fn install(
    tools: &ToolRegistry,
    manager: &PortForwardManager,
    permission: Arc<dyn ForwardPermission>,
) -> Result<(), RegisterToolError> {
    tools.register(Arc::new(ListPortForwardsTool::new(manager.clone())))?;
    tools.register(Arc::new(StartPortForwardTool::new(
        manager.clone(),
        permission,
    )))?;
    Ok(())
}
