//! The `node::Shell` handler: the guarded half of a node shell, then the request for a terminal.

use std::sync::Arc;

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};

use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::exec::ExecService;

/// Asks the UI to open the terminal of a node shell the guard allowed. Returns an error when
/// the window that would show it is gone; nothing was created then (the permit it leaves behind
/// expires on its own).
pub type NodeShellOpener = Arc<dyn Fn(&ResourceRef) -> OxiResult<()> + Send + Sync>;

/// Registers `node::Shell` on `registry` (with its MCP tool stub: unsafe, interactive, hidden
/// from agents by default), for the app's one [`ExecService`] and the UI's `open`.
///
/// The command is a mutation, so the bus has applied the read-only block, the confirmation (the
/// node and the image named) and the audit record by the time the handler runs; the handler
/// then dry-runs the pod through the guard's [`Mutation`](crate::Mutation) and leaves the permit
/// ([`ExecService::authorize_node_shell`]), and `open` queues the terminal, which exchanges the
/// permit for the session ([`ExecService::open_node_shell`]).
///
/// # Errors
///
/// A [`RegisterError`] when `node::Shell` is registered twice (a wiring bug).
pub fn register_command(
    registry: &mut CommandRegistry,
    service: Arc<ExecService>,
    open: NodeShellOpener,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::NODE_SHELL)
        .ok_or(RegisterError::Undeclared(CommandId::NODE_SHELL))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let (service, open) = (service.clone(), open.clone());
        async move {
            let Command::NodeShell { target } = command else {
                return Err(OxiError::validation("not a node shell command"));
            };
            let plan = service.authorize_node_shell(&cx, &target).await?;
            if plan.dry_run {
                return Ok(CommandOutput::message(format!(
                    "Dry run: the cluster would accept a privileged pod from {} in {} on {}",
                    plan.image, plan.namespace, plan.node
                )));
            }
            open(&target)?;
            Ok(CommandOutput::message(format!(
                "Starting a shell on {}",
                plan.node
            )))
        }
    })
}
