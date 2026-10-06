//! The `resource::Delete` handler.

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId, Propagation};
use oxikube_ports::{DeleteOptions, DeleteOutcome, PropagationPolicy};
use serde_json::json;

use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the `resource::Delete` handler on `registry` (the app installs it under
/// `oxikube_app::actions`).
///
/// The handler runs only after the guard let the command through: read-only check, confirmation
/// and audit precondition are the guard's. It deletes the object with the propagation the command
/// names (background, the kubectl default, unless the user chose another), after a server-side
/// dry run of the same delete: a request the server would refuse (forbidden, not found,
/// admission) fails before anything changes. A dispatch that is itself a dry run stops after the
/// dry run.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_DELETE)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_DELETE))?;
    registry.register(meta, |command: Command, cx: HandlerContext| async move {
        let Command::ResourceDelete {
            target,
            propagation,
        } = command
        else {
            return Err(OxiError::validation("not a resource::Delete command"));
        };
        let mutation = cx.require_mutation()?;
        let options = mutation.delete_options().propagation(policy(propagation));
        let ns = target.namespace();
        let writer = mutation.writer();
        let object = match ns {
            Some(ns) => format!("{} {ns}/{}", target.gvk.kind, target.name),
            None => format!("{} {}", target.gvk.kind, target.name),
        };
        if mutation.dry_run() {
            writer
                .delete(&target.gvk, ns, &target.name, &options)
                .await?;
            return Ok(CommandOutput {
                message: Some(format!("Dry run: {object} can be deleted")),
                data: Some(json!({ "outcome": "dry_run" })),
            });
        }
        // The server rehearses the delete first (admission, RBAC, existence).
        let rehearsal = DeleteOptions {
            dry_run: true,
            ..options.clone()
        };
        writer
            .delete(&target.gvk, ns, &target.name, &rehearsal)
            .await?;
        let outcome = writer
            .delete(&target.gvk, ns, &target.name, &options)
            .await?;
        Ok(match outcome {
            DeleteOutcome::Deleted => CommandOutput {
                message: Some(format!("Deleted {object}")),
                data: Some(json!({ "outcome": "deleted" })),
            },
            DeleteOutcome::Deleting(_) => CommandOutput {
                message: Some(format!("Deleting {object}")),
                data: Some(json!({ "outcome": "deleting" })),
            },
        })
    })
}

/// The API's propagation policy for the command's.
fn policy(propagation: Propagation) -> PropagationPolicy {
    match propagation {
        Propagation::Background => PropagationPolicy::Background,
        Propagation::Foreground => PropagationPolicy::Foreground,
        Propagation::Orphan => PropagationPolicy::Orphan,
    }
}
