//! The bus handlers of the workload commands.

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use serde_json::json;

use super::pod_commands::{self, POD_COMMANDS};
use super::service::WorkloadService;
use crate::actions::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::session::ClusterSessionManager;

/// The commands [`register_commands`] handles.
pub const WORKLOAD_COMMANDS: [CommandId; 5] = [
    CommandId::CRONJOB_TRIGGER,
    CommandId::CRONJOB_SUSPEND,
    CommandId::CRONJOB_RESUME,
    CommandId::JOB_SUSPEND,
    CommandId::JOB_RESUME,
];

/// Registers `cronjob::Trigger`, `cronjob::Suspend`, `cronjob::Resume`, `job::Suspend` and
/// `job::Resume`, `workload::Scale` (E12-S04), and the pod commands `pod::Evict`, `pod::Delete`,
/// `pod::ForceDelete` and `pod::Sanitize` (E12-S03), on `registry` (the binary installs them under
/// `oxikube_app::workloads`).
///
/// A handler runs only after the guard let the command through: the read-only check, the
/// confirmation and the audit precondition are the guard's. The trigger names the Job it made in
/// the audit record (`job=<name>`) and in its output.
///
/// # Errors
///
/// A [`RegisterError`] when one of the commands is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    super::scale_command::register_scale(registry, WorkloadService::new(sessions.clone()))?;
    register(
        registry,
        sessions,
        WORKLOAD_COMMANDS.into_iter().chain(POD_COMMANDS),
    )
}

/// Registers only the pod commands of [`register_commands`] (`pod::Evict`, `pod::Delete`,
/// `pod::ForceDelete`, `pod::Sanitize`), for a bus that handles the CronJob and Job commands
/// elsewhere (a test with stand-ins for them).
///
/// # Errors
///
/// A [`RegisterError`] when one of the commands is registered already.
pub fn register_pod_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    register(registry, sessions, POD_COMMANDS)
}

fn register(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
    ids: impl IntoIterator<Item = CommandId>,
) -> Result<(), RegisterError> {
    let service = WorkloadService::new(sessions);
    for id in ids {
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
) -> Result<CommandOutput, OxiError> {
    if POD_COMMANDS.contains(&command.id()) {
        return pod_commands::handle(service, command, cx).await;
    }
    let mutation = cx.require_mutation()?;
    match command {
        Command::CronJobTrigger { target } => {
            let triggered = service.trigger_cronjob(mutation, &target).await?;
            let job = triggered.job;
            cx.note_audit(format!("job={}", job.name));
            let (outcome, message) = if triggered.dry_run {
                (
                    "dry_run",
                    format!("Dry run: {} can be triggered", object_label(&target)),
                )
            } else {
                (
                    "triggered",
                    format!(
                        "Triggered {}: created {}",
                        object_label(&target),
                        object_label(&job)
                    ),
                )
            };
            Ok(CommandOutput {
                message: Some(message),
                data: Some(json!({
                    "outcome": outcome,
                    "target": target,
                    "job": job,
                    // Where a view goes next: the Job that now runs (nothing to open for a dry run).
                    "open": (!triggered.dry_run).then_some(&job),
                })),
            })
        }
        Command::CronJobSuspend { target } | Command::JobSuspend { target } => {
            let done = service.suspend(mutation, &target).await?;
            Ok(suspend_output(&target, true, done.dry_run))
        }
        Command::CronJobResume { target } | Command::JobResume { target } => {
            let done = service.resume(mutation, &target).await?;
            Ok(suspend_output(&target, false, done.dry_run))
        }
        other => Err(OxiError::validation(format!(
            "{} is not a workload command",
            other.id()
        ))),
    }
}

fn suspend_output(target: &ResourceRef, suspend: bool, dry_run: bool) -> CommandOutput {
    let label = object_label(target);
    let (done, verb) = if suspend {
        ("Suspended", "suspended")
    } else {
        ("Resumed", "resumed")
    };
    CommandOutput {
        message: Some(if dry_run {
            format!("Dry run: {label} can be {verb}")
        } else {
            format!("{done} {label}")
        }),
        data: Some(json!({
            "outcome": if dry_run { "dry_run" } else { verb },
            "target": target,
            "suspended": suspend,
        })),
    }
}
