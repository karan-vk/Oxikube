//! The `resource::Delete` handler.

use oxikube_domain::command::{self, Command, CommandId, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DeleteOptions, DeleteOutcome, PropagationPolicy};
use serde_json::json;

use super::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Mutation;

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
        let object = object_label(&target);
        let done = delete_object(mutation, &target, propagation).await?;
        Ok(match done {
            Deleted::DryRun => CommandOutput {
                message: Some(format!("Dry run: {object} can be deleted")),
                data: Some(json!({ "outcome": "dry_run" })),
            },
            Deleted::Gone => CommandOutput {
                message: Some(format!("Deleted {object}")),
                data: Some(json!({ "outcome": "deleted" })),
            },
            Deleted::Deleting => CommandOutput {
                message: Some(format!("Deleting {object}")),
                data: Some(json!({ "outcome": "deleting" })),
            },
        })
    })
}

/// What [`delete_object`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deleted {
    /// The dispatch was a dry run: the server accepted the request and nothing changed.
    DryRun,
    /// The object is gone.
    Gone,
    /// Deletion started but the object is still there (finalizers, foreground propagation).
    Deleting,
}

/// Deletes `target` with `propagation` through the guard's permit: a server-side dry run of the
/// same delete first (a request the server would refuse fails before anything changes; not when
/// the guard found the server cannot dry-run it, [`Mutation::rehearses_first`]), then the
/// delete, unless the dispatch is itself a dry run. The one place `resource::Delete` and
/// `selection::Delete` delete an object.
///
/// # Errors
///
/// The server's error: `NotFound`, `Forbidden`, an admission refusal.
pub(crate) async fn delete_object(
    mutation: &Mutation,
    target: &ResourceRef,
    propagation: Propagation,
) -> OxiResult<Deleted> {
    let options = mutation.delete_options().propagation(policy(propagation));
    let ns = target.namespace();
    let writer = mutation.writer();
    if mutation.dry_run() {
        writer
            .delete(&target.gvk, ns, &target.name, &options)
            .await?;
        return Ok(Deleted::DryRun);
    }
    // The server rehearses the delete first (admission, RBAC, existence), unless the guard found
    // it cannot dry-run one.
    if mutation.rehearses_first() {
        let rehearsal = DeleteOptions {
            dry_run: true,
            ..options.clone()
        };
        writer
            .delete(&target.gvk, ns, &target.name, &rehearsal)
            .await?;
    }
    Ok(
        match writer
            .delete(&target.gvk, ns, &target.name, &options)
            .await?
        {
            DeleteOutcome::Deleted => Deleted::Gone,
            DeleteOutcome::Deleting(_) => Deleted::Deleting,
        },
    )
}

/// The API's propagation policy for the command's.
fn policy(propagation: Propagation) -> PropagationPolicy {
    match propagation {
        Propagation::Background => PropagationPolicy::Background,
        Propagation::Foreground => PropagationPolicy::Foreground,
        Propagation::Orphan => PropagationPolicy::Orphan,
    }
}
