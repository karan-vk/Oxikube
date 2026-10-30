//! The `resource::Apply` handler: one object's apply, through the guard's [`Mutation`]. The write
//! itself is [`ObjectWrite`] (`write`), shared with the `resource::ApplyPlan` handler.
//!
//! ```text
//! dry-run dispatch:  read live ─> ObjectWrite::rehearse ─> preview data (live + result, masked)
//! real dispatch:     ObjectWrite::send (SSA dry run, SSA; or the replace / create fallback) ─> applied
//! ```
//!
//! The field manager is `oxikube` unless the command names one (the `editor.field_manager`
//! setting). A conflict with other managers comes back as a `Conflict` error with its
//! [`ConflictDetails`](oxikube_domain::ConflictDetails); a rejected field as a `Validation` error
//! with its causes. Secret values are masked in everything returned.
//!
//! [`Mutation`]: crate::guard::Mutation

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ResourceReader, ResourceWriter};
use serde_json::json;

use super::manifest::Manifest;
use super::mask::SecretMask;
use super::types::ApplyMethod;
use super::write::{ObjectWrite, Preview, version_of};
use crate::actions::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Rehearsal;
use crate::guard::no_preview_reason;
use crate::session::ClusterSessionManager;

/// The field manager when the command names none.
pub const DEFAULT_FIELD_MANAGER: &str = "oxikube";

/// Registers the `resource::Apply` and `resource::ApplyPlan` handlers on `registry` (the app
/// installs them under `oxikube_app::apply`). `sessions` gives them the cluster's reader (the live
/// object), discovery (whether the kind is namespaced) and clock (a new definition's wait).
///
/// # Errors
///
/// A [`RegisterError`] when a command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_APPLY)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_APPLY))?;
    let single = sessions.clone();
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let sessions = single.clone();
        async move { handle(command, cx, sessions).await }
    })?;
    super::plan::register(registry, sessions)
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
        reviewed_absent,
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
    let write = ObjectWrite {
        target: manifest.target(cluster, namespace.as_deref()),
        body: manifest.body(namespace.as_deref()),
        manager: field_manager
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_FIELD_MANAGER.to_owned()),
        force,
        resource_version,
        reviewed_absent,
    };
    let writer = mutation.writer();
    if mutation.dry_run() {
        return rehearse(&write, writer, reader.as_ref()).await;
    }
    // The guard dry-ran this apply before it was confirmed, or the person's own preview did
    // (E19-S07): no second dry run. Otherwise the server rehearses it first.
    let preview = match mutation.rehearsal() {
        Rehearsal::None => Preview::First,
        Rehearsal::Done => Preview::Done,
        Rehearsal::Unavailable => Preview::Unavailable,
    };
    let (method, object) = match write.send(writer, reader.as_ref(), preview).await {
        Ok(done) => done,
        // The API cannot dry-run the apply (an aggregated API, a webhook with side effects): sent
        // without the rehearsal; a missing preview never blocks what the user confirmed (E19-S07).
        Err(error) if preview == Preview::First && no_preview_reason(&error).is_some() => {
            write
                .send(writer, reader.as_ref(), Preview::Unavailable)
                .await?
        }
        Err(error) => return Err(error),
    };
    let label = object_label(&write.target);
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
            "target": write.target,
            "object": mask.apply(object.to_value()),
        })),
    })
}

/// The dry-run dispatch: the live object and the server's dry run of the apply (or of the
/// fallback), both masked with one key so they compare.
async fn rehearse(
    write: &ObjectWrite,
    writer: &dyn ResourceWriter,
    reader: &dyn ResourceReader,
) -> OxiResult<CommandOutput> {
    let target = &write.target;
    let live = write.live(reader).await?;
    let (method, object) = write.rehearse(writer, live.as_ref()).await?;
    let mask = SecretMask::new();
    Ok(CommandOutput {
        message: Some(format!("Dry run: {} can be applied", object_label(target))),
        data: Some(json!({
            "outcome": "dry_run",
            "method": method.as_str(),
            "target": target,
            "object": mask.apply(object.to_value()),
            "resource_version": version_of(live.as_ref()),
            "live": live.map(|live| mask.apply(live.to_value())),
        })),
    })
}
