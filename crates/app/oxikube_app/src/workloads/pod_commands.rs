//! The bus handlers of the pod commands (E12-S03): `pod::Evict`, `pod::Delete`,
//! `pod::ForceDelete` and `pod::Sanitize`.

use std::collections::HashSet;

use oxikube_domain::OxiError;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use serde_json::json;

use super::sanitize::new_batch_id;
use super::service::WorkloadService;
use crate::actions::object_label;
use crate::command_bus::{CommandOutput, HandlerContext};
use crate::guard::Mutation;

/// The pod commands the workloads module handles.
pub const POD_COMMANDS: [CommandId; 4] = [
    CommandId::POD_EVICT,
    CommandId::POD_DELETE,
    CommandId::POD_FORCE_DELETE,
    CommandId::POD_SANITIZE,
];

pub(super) async fn handle(
    service: &WorkloadService,
    command: Command,
    cx: &HandlerContext,
) -> Result<CommandOutput, OxiError> {
    let mutation = cx.require_mutation()?;
    match command {
        Command::PodEvict { target } => {
            let done = service.evict(mutation, &target).await?;
            let label = object_label(&target);
            Ok(output(
                if done.dry_run {
                    format!("Dry run: {label} can be evicted")
                } else {
                    format!("Evicted {label}")
                },
                if done.dry_run { "dry_run" } else { "evicted" },
                &target,
            ))
        }
        Command::PodDelete {
            target,
            grace_period_seconds,
        } => delete(service, cx, mutation, target, grace_period_seconds, false).await,
        Command::PodForceDelete { target } => {
            delete(service, cx, mutation, target, Some(0), true).await
        }
        Command::PodSanitize {
            cluster,
            namespace,
            targets,
        } => sanitize(service, cx, mutation, cluster, namespace, targets).await,
        other => Err(OxiError::validation(format!(
            "{} is not a pod command",
            other.id()
        ))),
    }
}

/// `pod::Sanitize`: deletes the named pods, or every Completed and Failed pod of the namespace (or
/// cluster) when none are named.
async fn sanitize(
    service: &WorkloadService,
    cx: &HandlerContext,
    mutation: &Mutation,
    cluster: ClusterId,
    namespace: Option<String>,
    targets: Vec<ResourceRef>,
) -> Result<CommandOutput, OxiError> {
    let pods = if targets.is_empty() {
        service
            .finished_pods(&cluster, namespace.as_deref())
            .await?
    } else {
        targets
    };
    // A pod named twice is swept once.
    let mut seen = HashSet::new();
    let pods: Vec<ResourceRef> = pods
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect();
    if pods.is_empty() {
        return Ok(CommandOutput {
            message: Some("No Completed or Failed pods to delete".to_owned()),
            data: Some(json!({"outcome": "nothing", "deleted": 0})),
        });
    }
    if pods
        .iter()
        .any(|pod| pod.cluster != cluster || !pod.gvk.is_pod())
    {
        return Err(OxiError::validation(
            "pod::Sanitize takes pods of its own cluster only",
        ));
    }
    let batch = new_batch_id();
    cx.note_audit(format!("batch={batch} pods={}", pods.len()));
    let sweep = service.delete_each(cx, mutation, pods, &batch).await?;
    if sweep.failed() > 0 {
        return Err(OxiError::conflict(format!(
            "{}: {}",
            sweep.summary(),
            sweep.failures().join("; ")
        )));
    }
    Ok(CommandOutput {
        message: Some(sweep.summary()),
        data: Some(json!({
            "outcome": if sweep.dry_run { "dry_run" } else { "deleted" },
            "batch": sweep.batch,
            "deleted": sweep.deleted(),
            "skipped": sweep.skipped(),
        })),
    })
}

async fn delete(
    service: &WorkloadService,
    cx: &HandlerContext,
    mutation: &Mutation,
    target: ResourceRef,
    grace: Option<u32>,
    force: bool,
) -> Result<CommandOutput, OxiError> {
    let done = service.delete_pod(mutation, &target, grace).await?;
    if let Some(grace) = grace {
        cx.note_audit(format!("grace={grace}"));
    }
    let label = object_label(&target);
    let (message, outcome) = match (done.dry_run, done.deleting, force) {
        (true, _, true) => (format!("Dry run: {label} can be force deleted"), "dry_run"),
        (true, _, false) => (format!("Dry run: {label} can be deleted"), "dry_run"),
        (false, _, true) => (format!("Force deleted {label}"), "deleted"),
        (false, true, false) => (format!("Deleting {label}"), "deleting"),
        (false, false, false) => (format!("Deleted {label}"), "deleted"),
    };
    Ok(output(message, outcome, &target))
}

fn output(message: String, outcome: &str, target: &ResourceRef) -> CommandOutput {
    CommandOutput {
        message: Some(message),
        data: Some(json!({ "outcome": outcome, "target": target })),
    }
}
