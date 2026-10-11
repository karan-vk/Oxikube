//! The guarded handlers of `namespace::Create` and `namespace::Delete`.
//!
//! Both run only after the [`MutationGuard`](crate::MutationGuard) let the command through:
//! read-only check, confirmation (one click for a create; the typed name for a delete, which is
//! irreversible) and the audit precondition. Both rehearse the request on the server first (a
//! `dryRun=All` create or delete), so a name that exists, a refusal by RBAC or by an admission
//! policy fails before anything changes; a dispatch that is itself a dry run stops there.

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_ports::{DeleteOutcome, WriteOptions};
use serde_json::json;

use super::input::{manifest, validate_labels, validate_name};
use crate::apply::DEFAULT_FIELD_MANAGER;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The kind both commands act on.
fn namespace_gvk() -> Gvk {
    Gvk::new("", "v1", "Namespace")
}

/// Registers `namespace::Create` and `namespace::Delete` on `registry`.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered already.
pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
    let create = *command::lookup(CommandId::NAMESPACE_CREATE)
        .ok_or(RegisterError::Undeclared(CommandId::NAMESPACE_CREATE))?;
    registry.register(create, |command: Command, cx: HandlerContext| async move {
        let Command::NamespaceCreate { name, labels, .. } = command else {
            return Err(OxiError::validation("not a namespace::Create command"));
        };
        validate_name(&name)?;
        validate_labels(&labels).map_err(|error| OxiError::validation(error.to_string()))?;
        let mutation = cx.require_mutation()?;
        let object = manifest(&name, &labels);
        let gvk = namespace_gvk();
        let options = |dry_run: bool| WriteOptions {
            dry_run,
            field_manager: Some(DEFAULT_FIELD_MANAGER.to_owned()),
        };
        let writer = mutation.writer();
        // The server rehearses the create first (a name that exists, RBAC, admission).
        writer.create(&gvk, None, &object, &options(true)).await?;
        if mutation.dry_run() {
            return Ok(CommandOutput {
                message: Some(format!("Dry run: namespace {name} can be created")),
                data: Some(json!({ "outcome": "dry_run", "name": name })),
            });
        }
        writer.create(&gvk, None, &object, &options(false)).await?;
        Ok(CommandOutput {
            message: Some(format!("Created namespace {name}")),
            data: Some(json!({ "outcome": "created", "name": name })),
        })
    })?;

    let delete = *command::lookup(CommandId::NAMESPACE_DELETE)
        .ok_or(RegisterError::Undeclared(CommandId::NAMESPACE_DELETE))?;
    registry.register(delete, |command: Command, cx: HandlerContext| async move {
        let Command::NamespaceDelete { target, .. } = command else {
            return Err(OxiError::validation("not a namespace::Delete command"));
        };
        if !(target.gvk.group.is_empty() && &*target.gvk.kind == "Namespace") {
            return Err(OxiError::validation(format!(
                "namespace::Delete acts on a Namespace, not {}",
                target.gvk.kind
            )));
        }
        let mutation = cx.require_mutation()?;
        let writer = mutation.writer();
        let options = mutation.delete_options();
        let name = target.name.to_string();
        if mutation.dry_run() {
            writer.delete(&target.gvk, None, &name, &options).await?;
            return Ok(CommandOutput {
                message: Some(format!("Dry run: namespace {name} can be deleted")),
                data: Some(json!({ "outcome": "dry_run", "name": name })),
            });
        }
        // The server rehearses the delete first (RBAC, admission, existence).
        let rehearsal = oxikube_ports::DeleteOptions {
            dry_run: true,
            ..options.clone()
        };
        writer.delete(&target.gvk, None, &name, &rehearsal).await?;
        let outcome = writer.delete(&target.gvk, None, &name, &options).await?;
        // A namespace is deleted asynchronously: it goes `Terminating` while its contents and
        // finalizers are removed.
        Ok(match outcome {
            DeleteOutcome::Deleted => CommandOutput {
                message: Some(format!("Deleted namespace {name}")),
                data: Some(json!({ "outcome": "deleted", "name": name })),
            },
            DeleteOutcome::Deleting(_) => CommandOutput {
                message: Some(format!(
                    "Namespace {name} is terminating: its contents are being removed"
                )),
                data: Some(json!({ "outcome": "deleting", "name": name })),
            },
        })
    })
}
