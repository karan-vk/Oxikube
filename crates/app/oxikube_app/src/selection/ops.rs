//! The work on one object of each bulk command.

use oxikube_domain::command::{CommandId, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::selection::{LabelEdit, LabelField, label_merge_patch};
use oxikube_ports::Patch;

use super::report::BulkStatus;
use super::rows::{Batch, settle};
use crate::actions::{Deleted, delete_object};
use crate::workloads::WorkloadService;

/// Deletes `target`, as `resource::Delete` (`pod::Delete` for a pod) would.
pub(super) async fn delete_row(
    batch: &Batch<'_>,
    target: &ResourceRef,
    propagation: Propagation,
) -> BulkStatus {
    let command = if target.gvk.is_pod() {
        CommandId::POD_DELETE
    } else {
        CommandId::RESOURCE_DELETE
    };
    batch
        .row(command, target, None, async {
            let result = delete_object(batch.mutation, target, propagation).await;
            let status = settle(result, |done| {
                BulkStatus::Ok(
                    match done {
                        Deleted::DryRun => "Can be deleted",
                        Deleted::Gone => "Deleted",
                        Deleted::Deleting => "Deleting",
                    }
                    .to_owned(),
                )
            });
            (status, None)
        })
        .await
}

/// Restarts `target`, as `workload::Restart` would.
pub(super) async fn restart_row(
    batch: &Batch<'_>,
    service: &WorkloadService,
    target: &ResourceRef,
) -> BulkStatus {
    batch
        .row(CommandId::WORKLOAD_RESTART, target, None, async {
            let note = std::sync::Mutex::new(None);
            let result = service
                .restart(batch.mutation, target, |text| {
                    if let Ok(mut slot) = note.lock() {
                        *slot = Some(text);
                    }
                })
                .await;
            let status = settle(result, |done| {
                BulkStatus::Ok(
                    if done.dry_run {
                        "Can be restarted"
                    } else {
                        "Restarting"
                    }
                    .to_owned(),
                )
            });
            (status, note.into_inner().ok().flatten())
        })
        .await
}

/// Applies the label (or annotation) patch to `target`: the server rehearses it, then it is sent.
pub(super) async fn label_row(
    batch: &Batch<'_>,
    target: &ResourceRef,
    field: LabelField,
    edits: &[LabelEdit],
) -> BulkStatus {
    let sets = edits.iter().filter(|edit| edit.value.is_some()).count();
    let detail = format!(
        "field={} set={sets} remove={}",
        field.key(),
        edits.len() - sets
    );
    batch
        .row(CommandId::SELECTION_LABEL, target, Some(&detail), async {
            let patch = Patch::merge(label_merge_patch(field, edits));
            let result = send(batch, target, &patch).await;
            let status = settle(result, |()| {
                BulkStatus::Ok(if batch.mutation.dry_run() {
                    format!("{}s can be changed", field.noun())
                } else {
                    format!("{}s changed", field.noun())
                })
            });
            (status, None)
        })
        .await
}

/// A server dry run of `patch` (admission, RBAC, existence), then the patch itself unless the
/// dispatch is a dry run.
async fn send(
    batch: &Batch<'_>,
    target: &ResourceRef,
    patch: &Patch,
) -> oxikube_domain::OxiResult<()> {
    let writer = batch.mutation.writer();
    let (namespace, name) = (target.namespace(), &*target.name);
    let options = |dry_run| oxikube_ports::WriteOptions {
        dry_run,
        ..oxikube_ports::WriteOptions::default().manager(crate::apply::DEFAULT_FIELD_MANAGER)
    };
    if !batch.mutation.dry_run() {
        writer
            .patch(&target.gvk, namespace, name, patch, &options(true))
            .await?;
    }
    writer
        .patch(
            &target.gvk,
            namespace,
            name,
            patch,
            &options(batch.mutation.dry_run()),
        )
        .await?;
    Ok(())
}
