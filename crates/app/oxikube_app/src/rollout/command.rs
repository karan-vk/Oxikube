//! The `workload::Rollback` handler.

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use serde_json::json;

use super::service::RolloutService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the `workload::Rollback` handler on `registry`.
///
/// The handler runs only after the guard let the command through (read-only check, confirmation,
/// audit precondition) and restores the revision with the guard's permit. A dispatch that is a dry
/// run has the server rehearse the patch and changes nothing.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub(crate) fn register_rollback(
    registry: &mut CommandRegistry,
    service: RolloutService,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::WORKLOAD_ROLLBACK)
        .ok_or(RegisterError::Undeclared(CommandId::WORKLOAD_ROLLBACK))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let service = service.clone();
        async move {
            let Command::WorkloadRollback {
                target,
                to_revision,
            } = command
            else {
                return Err(OxiError::validation("not a workload::Rollback command"));
            };
            let mutation = cx.require_mutation()?;
            let result = service
                .rollback(mutation, &target, to_revision, |note| cx.note_audit(note))
                .await?;
            Ok(CommandOutput {
                message: Some(result.message()),
                data: Some(json!({
                    "workload": result.workload,
                    "from_revision": result.from_revision,
                    "to_revision": result.to_revision,
                    "outcome": result.outcome.as_str(),
                })),
            })
        }
    })
}
