//! The `resource::Apply` handler: the only code that writes a manifest to a cluster, through the
//! guard's [`Mutation`].
//!
//! ```text
//! dry-run dispatch:  read live ─> SSA dry run ─unsupported─> replace / create dry run ─> preview data
//! real dispatch:     SSA dry run ─ok─> SSA ─────────────────────────────────────────────> applied
//!                         └─unsupported─> replace / create dry run ─> replace / create ─> applied
//! ```
//!
//! Server-side apply runs under the request's field manager (`oxikube` unless the
//! `editor.field_manager` setting says otherwise) with `force` only when the user chose it. A
//! server that cannot server-side apply the kind answers 415 (some aggregated APIs, old CRD
//! setups), which the adapter reports as `Unsupported`: the object is then replaced (PUT) carrying
//! the reviewed `resourceVersion`, so a change made since fails with a conflict instead of being
//! overwritten, or created when it does not exist. A conflict with other managers comes back as a
//! `Conflict` error with its [`ConflictDetails`](oxikube_domain::ConflictDetails); a rejected field
//! as a `Validation` error with its causes. Secret values are masked in everything returned.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_ports::{Patch, ResourceReader, ResourceWriter, WriteOptions};
use serde_json::{Value, json};

use super::manifest::{Manifest, with_resource_version};
use super::mask::SecretMask;
use super::types::ApplyMethod;
use crate::actions::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::session::ClusterSessionManager;

/// The field manager when the command names none.
pub const DEFAULT_FIELD_MANAGER: &str = "oxikube";

/// Registers the `resource::Apply` handler on `registry` (the app installs it under
/// `oxikube_app::apply`). `sessions` gives it the cluster's reader (the live object) and
/// discovery (whether the kind is namespaced).
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_APPLY)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_APPLY))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let sessions = sessions.clone();
        async move { handle(command, cx, sessions).await }
    })
}

/// One apply, as the command describes it.
struct Plan {
    target: ResourceRef,
    body: Value,
    manager: String,
    force: bool,
    resource_version: Option<String>,
}

impl Plan {
    fn namespace(&self) -> Option<&str> {
        self.target.namespace()
    }

    async fn server_side(&self, writer: &dyn ResourceWriter, dry_run: bool) -> OxiResult<Resource> {
        let patch = Patch::apply(self.body.clone(), self.manager.as_str(), self.force);
        let options = WriteOptions {
            dry_run,
            field_manager: Some(self.manager.clone()),
        };
        let target = &self.target;
        writer
            .patch(
                &target.gvk,
                self.namespace(),
                &target.name,
                &patch,
                &options,
            )
            .await
    }

    /// The fallback: a replace carrying `resource_version`, or a create without one.
    async fn fallback(
        &self,
        writer: &dyn ResourceWriter,
        resource_version: Option<&str>,
        dry_run: bool,
    ) -> OxiResult<(ApplyMethod, Resource)> {
        let options = WriteOptions {
            dry_run,
            field_manager: Some(self.manager.clone()),
        };
        let target = &self.target;
        match resource_version {
            Some(version) => {
                let body = with_resource_version(self.body.clone(), version);
                let object = writer
                    .replace(&target.gvk, self.namespace(), &target.name, &body, &options)
                    .await?;
                Ok((ApplyMethod::Replace, object))
            }
            None => {
                let object = writer
                    .create(&target.gvk, self.namespace(), &self.body, &options)
                    .await?;
                Ok((ApplyMethod::Create, object))
            }
        }
    }
}

async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::ResourceApply {
        cluster,
        namespace,
        manifest,
        field_manager,
        force,
        resource_version,
    } = command
    else {
        return Err(OxiError::validation("not a resource::Apply command"));
    };
    let mutation = cx.require_mutation()?;
    let manifest = Manifest::parse(&manifest)?;
    let session = sessions
        .get(&cluster)
        .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
    let reader = session
        .resources()
        .ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
    let namespace = manifest
        .resolve_namespace(&session, namespace.as_deref())
        .await;
    let plan = Plan {
        target: manifest.target(cluster, namespace.as_deref()),
        body: manifest.body(namespace.as_deref()),
        manager: field_manager
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_FIELD_MANAGER.to_owned()),
        force,
        resource_version,
    };
    let writer = mutation.writer();
    if mutation.dry_run() {
        return rehearse(&plan, writer, reader.as_ref()).await;
    }
    // The server rehearses the apply first (admission, validation, ownership); then it is sent.
    let (method, object) = match plan.server_side(writer, true).await {
        Ok(_) => (
            ApplyMethod::ServerSide,
            plan.server_side(writer, false).await?,
        ),
        Err(error) if error.kind() == ErrorKind::Unsupported => {
            let version = match plan.resource_version.clone() {
                Some(version) => Some(version),
                None => live_version(&plan, reader.as_ref()).await?,
            };
            plan.fallback(writer, version.as_deref(), true).await?;
            plan.fallback(writer, version.as_deref(), false).await?
        }
        Err(error) => return Err(error),
    };
    let label = object_label(&plan.target);
    let message = match method {
        ApplyMethod::ServerSide => format!("Applied {label}"),
        ApplyMethod::Replace => format!(
            "Replaced {label}: the server cannot server-side apply it, so it was replaced at the \
             reviewed version"
        ),
        ApplyMethod::Create => {
            format!("Created {label}: the server cannot server-side apply it")
        }
    };
    let mask = SecretMask::new();
    Ok(CommandOutput {
        message: Some(message),
        data: Some(json!({
            "outcome": "applied",
            "method": method.as_str(),
            "target": plan.target,
            "object": mask.apply(object.to_value()),
        })),
    })
}

/// The dry-run dispatch: the live object and the server's dry run of the apply (or of the
/// fallback), both masked with one key so they compare.
async fn rehearse(
    plan: &Plan,
    writer: &dyn ResourceWriter,
    reader: &dyn ResourceReader,
) -> OxiResult<CommandOutput> {
    let target = &plan.target;
    let live = reader
        .get_opt(&target.gvk, plan.namespace(), &target.name)
        .await?;
    let live_version = live
        .as_ref()
        .and_then(|live| live.meta.resource_version.as_deref())
        .map(str::to_owned);
    let (method, object) = match plan.server_side(writer, true).await {
        Ok(object) => (ApplyMethod::ServerSide, object),
        Err(error) if error.kind() == ErrorKind::Unsupported => {
            let version = plan.resource_version.clone().or(live_version.clone());
            let version = version.filter(|_| live.is_some());
            plan.fallback(writer, version.as_deref(), true).await?
        }
        Err(error) => return Err(error),
    };
    let mask = SecretMask::new();
    Ok(CommandOutput {
        message: Some(format!("Dry run: {} can be applied", object_label(target))),
        data: Some(json!({
            "outcome": "dry_run",
            "method": method.as_str(),
            "target": target,
            "object": mask.apply(object.to_value()),
            "live": live.map(|live| mask.apply(live.to_value())),
            "resource_version": live_version,
        })),
    })
}

/// The live object's version, read now (a fallback without a reviewed version); `None` when the
/// object does not exist.
async fn live_version(plan: &Plan, reader: &dyn ResourceReader) -> OxiResult<Option<String>> {
    let target = &plan.target;
    let live = reader
        .get_opt(&target.gvk, plan.namespace(), &target.name)
        .await?;
    Ok(live.and_then(|live| live.meta.resource_version.map(|v| v.to_string())))
}
