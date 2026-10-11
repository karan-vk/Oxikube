//! The confirmation text of the workload and pod commands (E12-S08, E12-S03): what happens
//! beyond the name. Names and counts only, never payload fields.

use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

use crate::actions::object_label;

/// The one-line confirmation of a workload or pod command, `None` for any other command.
pub(super) fn summary(command: &Command, context: &str) -> Option<String> {
    Some(match command {
        Command::CronJobTrigger { target } => format!(
            "Run {} now on {context}: a Job is created from its job template, as `kubectl create \
             job --from` does.",
            object_label(target)
        ),
        Command::CronJobSuspend { target } => format!(
            "Suspend {} on {context}: it creates no Jobs until it is resumed.",
            object_label(target)
        ),
        Command::JobSuspend { target } => format!(
            "Suspend {} on {context}: its running pods are deleted until it is resumed.",
            object_label(target)
        ),
        Command::CronJobResume { target } | Command::JobResume { target } => {
            format!("Resume {} on {context}.", object_label(target))
        }
        Command::WorkloadRestart { target } => format!(
            "Restart {} on {context}: its pods are replaced by new ones in a rolling update, as \
             `kubectl rollout restart` does.",
            object_label(target)
        ),
        Command::WorkloadSetImage { target, images } => {
            let changes = images
                .iter()
                .map(|change| format!("{} to {}", change.container, change.image))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Set the image of {} on {context}: {changes}. Its pods are replaced by new ones \
                 in a rolling update.",
                object_label(target)
            )
        }
        Command::PodEvict { target } => format!(
            "Evict {} on {context}: it is asked to leave through the eviction API, which \
             respects PodDisruptionBudgets, and its controller starts a replacement.",
            object_label(target)
        ),
        Command::PodDelete {
            target,
            grace_period_seconds,
        } => match grace_period_seconds {
            Some(0) => force_delete(target, context),
            Some(seconds) => format!(
                "Delete {} on {context}: it shuts down within {seconds} s and its controller, \
                 if any, starts a replacement.",
                object_label(target)
            ),
            // The ordinary delete reads as every other: "Delete Pod: Pod default/web-0 on prod".
            None => return None,
        },
        Command::PodForceDelete { target } => force_delete(target, context),
        Command::PodSanitize {
            namespace, targets, ..
        } => match (targets.len(), namespace) {
            (0, Some(namespace)) => format!(
                "Delete every Completed and Failed pod in namespace {namespace} on {context}."
            ),
            (0, None) => format!("Delete every Completed and Failed pod on {context}."),
            (1, _) => format!(
                "Delete 1 Completed or Failed pod on {context}: {}.",
                object_label(&targets[0])
            ),
            (count, _) => {
                format!("Delete {count} Completed and Failed pods on {context}.")
            }
        },
        _ => return None,
    })
}

/// Grace period 0 removes the API object without waiting for the kubelet.
fn force_delete(target: &ResourceRef, context: &str) -> String {
    format!(
        "Force delete {} on {context} (grace period 0): the pod leaves the API at once without \
         waiting for the kubelet, so its containers may keep running on a node that cannot be \
         reached.",
        object_label(target)
    )
}
