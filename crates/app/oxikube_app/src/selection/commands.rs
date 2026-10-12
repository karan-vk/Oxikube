//! The `selection::Delete`, `selection::Restart` and `selection::Label` handlers.

use std::collections::HashSet;

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::selection::validate_edits;
use oxikube_domain::{OxiError, OxiResult};

use super::ops::{delete_row, label_row, restart_row};
use super::report::{BulkReport, BulkRow, BulkStatus};
use super::rows::Batch;
use crate::actions::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Mutation;
use crate::session::ClusterSessionManager;
use crate::workloads::{CONCURRENCY, WorkloadService, is_rollable, new_batch_id, run_each};

/// The commands [`register_commands`] handles.
pub const SELECTION_COMMANDS: [CommandId; 3] = [
    CommandId::SELECTION_DELETE,
    CommandId::SELECTION_RESTART,
    CommandId::SELECTION_LABEL,
];

/// How many objects one bulk command takes: a view never lists more than this, and neither does a
/// batch. A bigger selection is refused up front, before anything is sent.
pub const MAX_BATCH: usize = 10_000;

/// Registers the three bulk handlers on `registry` (the binary installs them under
/// `oxikube_app::selection`).
///
/// A handler runs only after the guard let the command through (read-only check for the whole
/// batch, the one confirmation at the riskiest object's tier, the audit precondition). It then
/// works on the objects a few at a time, never stops at a failure, and writes one audit record per
/// object under a shared `batch=<id>`.
///
/// # Errors
///
/// A [`RegisterError`] when one of the commands is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let service = WorkloadService::new(sessions);
    for id in SELECTION_COMMANDS {
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
) -> OxiResult<CommandOutput> {
    let mutation = cx.require_mutation()?;
    let id = command.id();
    let targets = checked_targets(command.targets(), mutation)?;
    let batch_id = new_batch_id("selection");
    let batch = Batch {
        cx,
        mutation,
        id: &batch_id,
        via: id,
    };
    let verb = check(&command, &targets)?;
    let rows = run_each(targets, CONCURRENCY, |target| {
        let (batch, command) = (&batch, &command);
        async move {
            let status = work(batch, service, command, &target).await;
            BulkRow { target, status }
        }
    })
    .await;
    let report = BulkReport {
        batch: batch_id,
        dry_run: mutation.dry_run(),
        verb: verb.to_owned(),
        rows,
    };
    cx.note_audit(format!(
        "batch={} ok={} skipped={} failed={}",
        report.batch,
        report.succeeded(),
        report.skipped(),
        report.failed()
    ));
    Ok(CommandOutput {
        message: Some(report.summary()),
        data: Some(report.to_data()),
    })
}

/// Checks what the command asks of every object before the first request and returns the word
/// its report uses for what it did.
fn check(command: &Command, targets: &[ResourceRef]) -> OxiResult<&'static str> {
    match command {
        Command::SelectionDelete { .. } => Ok("deleted"),
        Command::SelectionRestart { .. } => {
            if let Some(other) = targets.iter().find(|target| !is_rollable(&target.gvk)) {
                return Err(OxiError::validation(format!(
                    "{} cannot be restarted: only Deployments, StatefulSets and DaemonSets can.",
                    object_label(other)
                )));
            }
            Ok("restarted")
        }
        Command::SelectionLabel { field, edits, .. } => {
            validate_edits(*field, edits)?;
            Ok("labelled")
        }
        other => Err(OxiError::validation(format!(
            "{} is not a bulk command",
            other.id()
        ))),
    }
}

/// The work on one object of the batch.
async fn work(
    batch: &Batch<'_>,
    service: &WorkloadService,
    command: &Command,
    target: &ResourceRef,
) -> BulkStatus {
    match command {
        Command::SelectionDelete { propagation, .. } => {
            delete_row(batch, target, *propagation).await
        }
        Command::SelectionRestart { .. } => restart_row(batch, service, target).await,
        Command::SelectionLabel { field, edits, .. } => {
            label_row(batch, target, *field, edits).await
        }
        other => BulkStatus::Failed(format!("{} is not a bulk command", other.id())),
    }
}

/// The objects of the batch: at least one, a few thousand at most, all in the permit's cluster,
/// each once (a repeated object keeps its first place).
fn checked_targets(targets: &[ResourceRef], mutation: &Mutation) -> OxiResult<Vec<ResourceRef>> {
    if targets.is_empty() {
        return Err(OxiError::validation("nothing is selected"));
    }
    if targets.iter().any(|t| &t.cluster != mutation.cluster()) {
        return Err(OxiError::validation(
            "the selection belongs to more than one cluster",
        ));
    }
    let mut seen = HashSet::with_capacity(targets.len());
    let unique: Vec<ResourceRef> = targets
        .iter()
        .filter(|target| seen.insert(*target))
        .cloned()
        .collect();
    if unique.len() > MAX_BATCH {
        return Err(OxiError::validation(format!(
            "{} objects are selected; a bulk action takes at most {MAX_BATCH}",
            unique.len()
        )));
    }
    Ok(unique)
}
