//! The `resource::CopyTo` handler: the source object read, stripped, renamed and written to the
//! target cluster through the guard's [`Mutation`](crate::guard::Mutation).
//!
//! ```text
//! source cluster:  get ─────────────────────────────> audit record (read)
//! target cluster:  kind / apiVersion served? namespace exists? object exists?
//!                  ObjectWrite::rehearse (dry-run dispatch) | ObjectWrite::send (real)
//! ```
//!
//! The write is the apply pipeline's [`ObjectWrite`]: server-side apply under the field manager
//! `oxikube`, the dry run first, the replace / create fallback. This handler holds the target's
//! permit (the target's read-only flag, confirmation and audit were the guard's); the source is
//! only read through a [`ResourceReader`](oxikube_ports::ResourceReader). Secret values are
//! masked in everything returned.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ConflictDetails, ConflictReason, OxiError, OxiResult};
use serde_json::{Value, json};

use super::audit::SourceRead;
use super::object::prepare;
use super::references;
use super::refusal::{Refusal, validate_name};
use super::target::{namespace_exists, served_kind, unavailable};
use crate::apply::DEFAULT_FIELD_MANAGER;
use crate::apply::mask::SecretMask;
use crate::apply::write::{ObjectWrite, version_of};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::session::ClusterSessionManager;

/// Registers the `resource::CopyTo` handler on `registry` (the app installs it under
/// `oxikube_app::copy`). `sessions` gives it the source and the target cluster's ports.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::RESOURCE_COPY_TO)
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_COPY_TO))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let sessions = sessions.clone();
        async move { handle(command, cx, sessions).await }
    })
}

/// Whether `gvk` names are labels (63 characters, no dots) rather than subdomains.
pub(super) fn name_is_label(gvk: &Gvk) -> bool {
    gvk.is_core() && matches!(&*gvk.kind, "Namespace" | "Service")
}

async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::ResourceCopyTo {
        source,
        target,
        overwrite,
        force,
    } = command
    else {
        return Err(OxiError::validation("not a resource::CopyTo command"));
    };
    let mutation = cx.require_mutation()?;
    validate_name(&target.name, name_is_label(&target.gvk))
        .map_err(|message| Refusal::InvalidName(message).into_error())?;
    let source_session = sessions
        .get(&source.cluster)
        .ok_or_else(|| OxiError::not_found("the source cluster is not open"))?;
    let source_reader = source_session
        .resources()
        .ok_or_else(|| unavailable("the source cluster"))?;
    let target_session = sessions
        .get(&target.cluster)
        .ok_or_else(|| OxiError::not_found("the target cluster is not open"))?;
    let target_reader = target_session
        .resources()
        .ok_or_else(|| unavailable("the target cluster"))?;
    let discovery = target_session
        .discovery()
        .ok_or_else(|| unavailable("the target cluster"))?;

    let object = read_source(&cx, &source, &target, source_reader.as_ref()).await?;
    let kind = served_kind(discovery.as_ref(), &source.gvk, target_session.title()).await?;
    let target = ResourceRef::new(
        target.cluster.clone(),
        source.gvk.clone(),
        kind.namespaced
            .then(|| target.namespace().map(Into::into))
            .flatten(),
        &*target.name,
    );
    let prepared = prepare(object, &target, kind.namespaced)?;
    namespace_exists(target_reader.as_ref(), &target).await?;

    let mut write = ObjectWrite {
        target: target.clone(),
        body: prepared.body,
        manager: DEFAULT_FIELD_MANAGER.to_owned(),
        force,
        resource_version: None,
        reviewed_absent: false,
    };
    let live = write.live(target_reader.as_ref()).await?;
    if live.is_some() && !overwrite {
        return Err(exists(&target));
    }
    write.resource_version = version_of(live.as_ref());
    write.reviewed_absent = live.is_none();
    let missing =
        references::missing(&write.body, target.namespace(), target_reader.as_ref()).await;
    cx.note_audit(format!(
        "source_context={} sanitized={} missing_references={}",
        source_session.context(),
        prepared.removed.len(),
        missing.len()
    ));

    let mask = SecretMask::new();
    let warnings: Vec<Value> = missing
        .iter()
        .map(|reference| {
            json!({
                "kind": reference.kind,
                "name": reference.name,
                "namespace": reference.namespace,
            })
        })
        .collect();
    let (method, stored) = if mutation.dry_run() {
        write.rehearse(mutation.writer(), live.as_ref()).await?
    } else {
        write
            .send(mutation.writer(), target_reader.as_ref())
            .await?
    };
    let label = crate::actions::object_label(&target);
    let created = live.is_none();
    let message = match (mutation.dry_run(), created) {
        (true, _) => format!("Dry run: {label} can be copied"),
        (false, true) => format!("Created {label} on {}", target_session.title()),
        (false, false) => format!("Overwrote {label} on {}", target_session.title()),
    };
    Ok(CommandOutput {
        message: Some(message),
        data: Some(json!({
            "outcome": if mutation.dry_run() { "dry_run" } else { "copied" },
            "method": method.as_str(),
            "target": target,
            "created": created,
            "object": mask.apply(stored.to_value()),
            "live": live.map(|live| mask.apply(live.to_value())),
            "resource_version": write.resource_version,
            "removed": prepared.removed,
            "missing": warnings,
        })),
    })
}

/// Reads the source object, with a record of the read in the source cluster's audit log.
async fn read_source(
    cx: &HandlerContext,
    source: &ResourceRef,
    target: &ResourceRef,
    reader: &dyn oxikube_ports::ResourceReader,
) -> OxiResult<Value> {
    let read = cx.audit().map(|audit| {
        SourceRead::begin(
            audit,
            cx.who(),
            cx.initiator(),
            source,
            target,
            cx.mutation().is_some_and(|mutation| mutation.dry_run()),
        )
    });
    let result = reader
        .get(&source.gvk, source.namespace(), &source.name)
        .await;
    if let Some(read) = read {
        read.finish(result.is_ok()).await;
    }
    match result {
        Ok(object) => Ok(object.to_value()),
        Err(error) => Err(Refusal::SourceUnavailable(format!(
            "could not read {} from the source cluster: {}",
            crate::actions::object_label(source),
            error.message()
        ))
        .into_error()),
    }
}

/// The conflict for an object that exists at the target when the copy may not overwrite it.
fn exists(target: &ResourceRef) -> OxiError {
    OxiError::conflict(format!(
        "{} exists on the target already",
        crate::actions::object_label(target)
    ))
    .with_source(ConflictDetails {
        reason: ConflictReason::AlreadyExists,
        causes: Vec::new(),
    })
}
