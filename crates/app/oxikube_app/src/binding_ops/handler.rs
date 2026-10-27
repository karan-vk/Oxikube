//! The `binding::Create`, `binding::Edit` and `binding::Delete` handlers.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::rbac::RbacKind;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::WriteOptions;
use serde_json::{Value, json};

use super::BindingManifest;
use crate::actions::{Deleted, delete_object, object_label};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Mutation;

/// Registers `binding::Create`, `binding::Edit` and `binding::Delete` on `registry` (tool stubs
/// `k8s.binding_create`, `k8s.binding_edit`, `k8s.binding_delete`). The binary installs it as
/// `oxikube_app::binding_ops`.
///
/// The guard runs first (read-only check, simple confirmation, audit). A create or edit then
/// asks the server for a dry run, and sends the write only when that succeeds and the dispatch is
/// not itself a dry run. Output `data` is
/// `{"outcome": "created" | "updated" | "recreated" | "deleted" | "deleting" | "dry_run", ...}`.
///
/// # Errors
///
/// A [`RegisterError`] when any of the commands is registered already.
pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
    for id in [
        CommandId::BINDING_CREATE,
        CommandId::BINDING_EDIT,
        CommandId::BINDING_DELETE,
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
        Command::BindingCreate { cluster, manifest } => {
            let binding = BindingManifest::parse(&manifest)?;
            let object = write_checked(mutation, Write::Create, &binding).await?;
            let target = binding.target(&cluster);
            Ok(written("created", "Created", &target, object, mutation))
        }
        Command::BindingEdit {
            cluster,
            manifest,
            previous: None,
        } => {
            let binding = BindingManifest::parse(&manifest)?;
            if binding.resource_version.is_none() {
                return Err(OxiError::validation(
                    "the edited binding carries no metadata.resourceVersion, so a change made \
                     since it was read would be overwritten",
                ));
            }
            let object = write_checked(mutation, Write::Replace, &binding).await?;
            let target = binding.target(&cluster);
            Ok(written("updated", "Updated", &target, object, mutation))
        }
        Command::BindingEdit {
            cluster,
            manifest,
            previous: Some(previous),
        } => {
            let binding = BindingManifest::parse(&manifest)?;
            let before = BindingManifest::parse(&previous)?;
            if (binding.kind, &binding.namespace, &binding.name)
                != (before.kind, &before.namespace, &before.name)
            {
                return Err(OxiError::validation(
                    "a new roleRef replaces the binding under the same kind, namespace and name",
                ));
            }
            recreate(mutation, &cluster, &before, &binding).await
        }
        Command::BindingDelete { target } => {
            let is_binding = matches!(
                RbacKind::of(&target.gvk),
                Some(RbacKind::RoleBinding | RbacKind::ClusterRoleBinding)
            );
            if !is_binding {
                return Err(OxiError::validation(
                    "binding::Delete deletes a RoleBinding or ClusterRoleBinding",
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
            "not a binding::Create, binding::Edit or binding::Delete command",
        )),
    }
}

/// How a binding is written.
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
    binding: &BindingManifest,
    body: &Value,
    options: &WriteOptions,
) -> OxiResult<Resource> {
    let (gvk, namespace) = (binding.gvk(), binding.namespace.as_deref());
    match how {
        Write::Create => {
            mutation
                .writer()
                .create(&gvk, namespace, body, options)
                .await
        }
        Write::Replace => {
            mutation
                .writer()
                .replace(&gvk, namespace, &binding.name, body, options)
                .await
        }
    }
}

/// Sends `binding` as a dry run, then for real unless the dispatch is itself a dry run; returns
/// the object of the last request.
async fn write_checked(
    mutation: &Mutation,
    how: Write,
    binding: &BindingManifest,
) -> OxiResult<Value> {
    let body = match how {
        Write::Create => binding.for_create(),
        Write::Replace => binding.body.clone(),
    };
    let rehearsal = WriteOptions {
        dry_run: true,
        ..mutation.write_options()
    };
    let rehearsed = send(mutation, how, binding, &body, &rehearsal).await?;
    if mutation.dry_run() {
        return Ok(rehearsed.to_value());
    }
    Ok(
        send(mutation, how, binding, &body, &mutation.write_options())
            .await?
            .to_value(),
    )
}

/// A changed `roleRef`: delete `before`, create `after`, and put `before` back if `after` is
/// refused. A dry-run dispatch rehearses the delete only (the name is taken until it happens).
async fn recreate(
    mutation: &Mutation,
    cluster: &oxikube_domain::ids::ClusterId,
    before: &BindingManifest,
    after: &BindingManifest,
) -> OxiResult<CommandOutput> {
    let target = after.target(cluster);
    let old = before.target(cluster);
    let done = delete_object(mutation, &old, command::Propagation::Background).await?;
    if matches!(done, Deleted::DryRun) {
        return Ok(written(
            "dry_run",
            "",
            &target,
            after.body.clone(),
            mutation,
        ));
    }
    match write_checked(mutation, Write::Create, after).await {
        Ok(object) => Ok(CommandOutput {
            message: Some(format!(
                "Recreated {} with {}",
                object_label(&target),
                after.role_label()
            )),
            data: Some(json!({ "outcome": "recreated", "target": target, "object": object })),
        }),
        Err(error) => {
            let restored = write_checked(mutation, Write::Create, before).await;
            Err(OxiError::new(
                error.kind(),
                match restored {
                    Ok(_) => format!(
                        "{}; the old binding ({}) was put back",
                        error.message(),
                        before.role_label()
                    ),
                    Err(why) => format!(
                        "{}; and the old binding ({}) could not be put back: {}",
                        error.message(),
                        before.role_label(),
                        why.message()
                    ),
                },
            ))
        }
    }
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
