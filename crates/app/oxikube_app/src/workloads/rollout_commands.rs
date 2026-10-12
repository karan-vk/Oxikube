//! The `workload::Restart` and `workload::SetImage` handlers, installed by
//! [`register_commands`](super::register_commands).

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use serde_json::json;

use super::service::WorkloadService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the two handlers on `registry`.
///
/// A handler runs only after the guard let the command through (read-only check, confirmation,
/// audit precondition) and patches with the guard's permit. A dispatch that is a dry run has the
/// server rehearse the patch and changes nothing. Neither waits for the rollout: the output tells
/// a view to open the workload (`data.open`), whose panel follows the rollout live.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered already.
pub(super) fn register_rollout(
    registry: &mut CommandRegistry,
    service: WorkloadService,
) -> Result<(), RegisterError> {
    for id in [CommandId::WORKLOAD_RESTART, CommandId::WORKLOAD_SET_IMAGE] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let service = service.clone();
        registry.register(meta, move |command: Command, cx: HandlerContext| {
            let service = service.clone();
            async move { handle(&service, command, &cx).await }
        })?;
    }
    Ok(())
}

async fn handle(
    service: &WorkloadService,
    command: Command,
    cx: &HandlerContext,
) -> Result<CommandOutput, OxiError> {
    let mutation = cx.require_mutation()?;
    match command {
        Command::WorkloadRestart { target } => {
            let done = service
                .restart(mutation, &target, |note| cx.note_audit(note))
                .await?;
            Ok(CommandOutput {
                message: Some(done.message()),
                data: Some(json!({
                    "outcome": if done.dry_run { "dry_run" } else { "restarting" },
                    "target": target,
                    "restarted_at": done.stamp(),
                    "resource_version": done.resource_version,
                    // Where a view goes next: the workload, whose panel shows the rollout.
                    "open": (!done.dry_run).then_some(&target),
                })),
            })
        }
        Command::WorkloadSetImage { target, images } => {
            let done = service
                .set_image(mutation, &target, &images, |note| cx.note_audit(note))
                .await?;
            Ok(CommandOutput {
                message: Some(done.message()),
                data: Some(json!({
                    "outcome": if done.dry_run { "dry_run" } else { "image_set" },
                    "target": target,
                    "changes": done.edits.iter().map(|edit| json!({
                        "container": &*edit.container,
                        "init": edit.init,
                        "from": &*edit.from,
                        "to": &*edit.to,
                    })).collect::<Vec<_>>(),
                    "resource_version": done.resource_version,
                    "open": (!done.dry_run).then_some(&target),
                })),
            })
        }
        other => Err(OxiError::validation(format!(
            "{} is not a rollout command",
            other.id()
        ))),
    }
}
