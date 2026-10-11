//! The `workload::Scale` handler, installed by [`register_commands`](super::register_commands).

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use serde_json::json;

use super::service::WorkloadService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the `workload::Scale` handler on `registry`.
///
/// The handler runs only after the guard let the command through (read-only check, confirmation,
/// audit precondition) and scales through the `scale` subresource with the guard's permit. A
/// dispatch that is a dry run has the server rehearse the patch and changes nothing.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub(super) fn register_scale(
    registry: &mut CommandRegistry,
    service: WorkloadService,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::WORKLOAD_SCALE)
        .ok_or(RegisterError::Undeclared(CommandId::WORKLOAD_SCALE))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let service = service.clone();
        async move {
            let Command::WorkloadScale { target, replicas } = command else {
                return Err(OxiError::validation("not a workload::Scale command"));
            };
            let mutation = cx.require_mutation()?;
            let result = service
                .scale(mutation, &target, replicas, |note| cx.note_audit(note))
                .await?;
            Ok(CommandOutput {
                message: Some(result.message()),
                data: Some(json!({
                    "before": result.before,
                    "after": result.after,
                    "dry_run": result.dry_run,
                })),
            })
        }
    })
}
