//! The `role::Create`, `role::Edit` and `role::Delete` handlers.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::rbac::RbacKind;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::WriteOptions;
use serde_json::{Value, json};

use super::RoleManifest;
use crate::actions::{Deleted, delete_object, object_label};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Mutation;

/// Registers `role::Create`, `role::Edit` and `role::Delete` on `registry` (tool stubs
/// `k8s.role_create`, `k8s.role_edit`, `k8s.role_delete`). The binary installs it as
/// `oxikube_app::role_ops`.
///
/// The guard runs first (read-only check, simple confirmation, audit). A create or edit then
/// asks the server for a dry run, and sends the write only when that succeeds and the dispatch is
/// not itself a dry run. Output `data` is
/// `{"outcome": "created" | "updated" | "deleted" | "deleting" | "dry_run", ...}`.
///
/// # Errors
///
/// A [`RegisterError`] when any of the commands is registered already.
pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
    for id in [
        CommandId::ROLE_CREATE,
        CommandId::ROLE_EDIT,
        CommandId::ROLE_DELETE,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        registry.register(meta, |command: Command, cx: HandlerContext| async move {
            let mutation = cx.require_mutation()?;
            run(mutation, command).await
        })?;
    }
    Ok(())
}

async fn run(mutation: &Mutation, command: Command) -> OxiResult<CommandOutput> {
    match command {
        Command::RoleCreate { cluster, manifest } => {
            let role = RoleManifest::parse(&manifest)?;
            let object = write_checked(mutation, Write::Create, &role).await?;
            let target = role.target(&cluster);
            Ok(written(
                "created",
                "Created",
                &target,
                object.to_value(),
                mutation,
            ))
        }
        Command::RoleEdit { cluster, manifest } => {
            let role = RoleManifest::parse(&manifest)?;
            if role.resource_version.is_none() {
                return Err(OxiError::validation(
                    "the edited role carries no metadata.resourceVersion, so a change made since \
                     it was read would be overwritten",
                ));
            }
            let object = write_checked(mutation, Write::Replace, &role).await?;
            let target = role.target(&cluster);
            Ok(written(
                "updated",
                "Updated",
                &target,
                object.to_value(),
                mutation,
            ))
        }
        Command::RoleDelete { target, .. } => {
            let is_role = matches!(
                RbacKind::of(&target.gvk),
                Some(RbacKind::Role | RbacKind::ClusterRole)
            );
            if !is_role {
                return Err(OxiError::validation(
                    "role::Delete deletes a Role or ClusterRole",
                ));
            }
            let label = object_label(&target);
            let done = delete_object(mutation, &target, command::Propagation::Background).await?;
            Ok(match done {
                Deleted::DryRun => CommandOutput {
                    message: Some(format!("Dry run: {label} can be deleted")),
                    data: Some(json!({ "outcome": "dry_run" })),
                },
                Deleted::Gone => CommandOutput {
                    message: Some(format!("Deleted {label}")),
                    data: Some(json!({ "outcome": "deleted" })),
                },
                Deleted::Deleting => CommandOutput {
                    message: Some(format!("Deleting {label}")),
                    data: Some(json!({ "outcome": "deleting" })),
                },
            })
        }
        _ => Err(OxiError::validation(
            "not a role::Create, role::Edit or role::Delete command",
        )),
    }
}

/// How a role is written.
#[derive(Debug, Clone, Copy)]
enum Write {
    /// POST a new object.
    Create,
    /// PUT over the existing one, at the manifest's `resourceVersion`.
    Replace,
}

async fn send(
    mutation: &Mutation,
    how: Write,
    role: &RoleManifest,
    options: &WriteOptions,
) -> OxiResult<Resource> {
    let (gvk, namespace) = (role.gvk(), role.namespace.as_deref());
    match how {
        Write::Create => {
            mutation
                .writer()
                .create(&gvk, namespace, &role.body, options)
                .await
        }
        Write::Replace => {
            mutation
                .writer()
                .replace(&gvk, namespace, &role.name, &role.body, options)
                .await
        }
    }
}

/// Sends `role` as a dry run, then for real unless the dispatch is itself a dry run; returns the
/// object of the last request.
async fn write_checked(
    mutation: &Mutation,
    how: Write,
    role: &RoleManifest,
) -> OxiResult<Resource> {
    let rehearsal = WriteOptions {
        dry_run: true,
        ..mutation.write_options()
    };
    let rehearsed = send(mutation, how, role, &rehearsal).await?;
    if mutation.dry_run() {
        return Ok(rehearsed);
    }
    send(mutation, how, role, &mutation.write_options()).await
}

fn written(
    outcome: &str,
    verb: &str,
    target: &ResourceRef,
    object: Value,
    mutation: &Mutation,
) -> CommandOutput {
    let label = object_label(target);
    if mutation.dry_run() {
        return CommandOutput {
            message: Some(format!("Dry run: {label} can be written")),
            data: Some(json!({ "outcome": "dry_run", "target": target, "object": object })),
        };
    }
    CommandOutput {
        message: Some(format!("{verb} {label}")),
        data: Some(json!({ "outcome": outcome, "target": target, "object": object })),
    }
}
