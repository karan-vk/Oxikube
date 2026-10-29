//! Turning the guard's [`ConfirmationRequest`] into a [`ConfirmPrompt`]: the words, the cluster,
//! the dry-run banner, the objects the dry run would write and the server's warnings (E19-S07),
//! and the objects of a bulk request.

use oxikube_app::{ClusterSession, ConfirmationRequest, DryRunPreview, object_label};
use oxikube_domain::command::{Command, delete_risk};
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::{ConfirmItem, ConfirmPrompt, PreviewNote};

/// The title, the confirm button and whether it is styled destructive, for the dialog the guard's
/// request opens. The posture commands (read-only, colour, presets) ask "Are you sure?" and offer
/// to turn the protection off; a debug container (E09-S10) is an addition that cannot be undone;
/// a node shell (E09-S09) says what is being opened, because it creates a privileged pod; the
/// workload commands (E12-S08) name their verb on the button.
fn confirm_copy(command: &Command) -> (&'static str, &'static str, bool) {
    match command {
        Command::PodDebug { .. } => ("Add a debug container?", "Add debug container", false),
        Command::NodeShell { .. } => ("Open a shell on the node?", "Open shell", true),
        Command::CronJobTrigger { .. } => ("Run the CronJob now?", "Trigger", false),
        Command::CronJobSuspend { .. } | Command::JobSuspend { .. } => {
            ("Suspend it?", "Suspend", false)
        }
        Command::CronJobResume { .. } | Command::JobResume { .. } => {
            ("Resume it?", "Resume", false)
        }
        Command::PodEvict { .. } => ("Evict the pod?", "Evict", true),
        Command::PodDelete { .. } => ("Delete the pod?", "Delete", true),
        Command::PodForceDelete { .. } => ("Force delete the pod?", "Force delete", true),
        Command::PodSanitize { .. } => ("Delete finished pods?", "Delete", true),
        Command::WorkloadRestart { .. } => ("Restart the workload?", "Restart", false),
        Command::WorkloadSetImage { .. } => ("Set the image?", "Set image", false),
        Command::BindingCreate { .. } => ("Create the binding?", "Create", false),
        Command::BindingEdit { .. } => ("Save the binding?", "Save", false),
        Command::BindingDelete { .. } => ("Delete the binding?", "Delete", true),
        Command::RoleCreate { .. } => ("Create the role?", "Create", false),
        Command::RoleEdit { .. } => ("Save the role?", "Save", false),
        Command::RoleDelete { .. } => ("Delete the role?", "Delete", true),
        _ => ("Are you sure?", "Turn off", true),
    }
}

/// The prompt for `request`, the guard's question about `command`. `session` is the cluster's
/// session, for its name and colour; without one the cluster id stands in for the name.
pub(crate) fn prompt_for(
    command: &Command,
    request: &ConfirmationRequest,
    session: Option<&ClusterSession>,
) -> ConfirmPrompt {
    let (title, confirm_label, destructive) = confirm_copy(command);
    let name = session.map_or_else(
        || request.cluster.to_string(),
        |session| {
            session
                .display_name()
                .map_or_else(|| session.context().to_string(), ToOwned::to_owned)
        },
    );
    let mut prompt = ConfirmPrompt::new(title, request.summary.clone())
        .with_confirm_label(confirm_label)
        .with_destructive(destructive)
        .with_cluster(name, session.and_then(ClusterSession::colour))
        .with_initiator(request.initiator)
        .with_risk(request.risk)
        .with_preview(request.preview.as_ref().map(preview_note))
        .with_items(items_of(command, request.risk));
    if let Some(DryRunPreview::Rehearsed(result)) = &request.preview {
        prompt = prompt
            .with_changes(result.changes.clone(), result.omitted)
            .with_warnings(result.warnings.iter().map(|w| w.clone().into()).collect());
    }
    if request.tier == ConfirmTier::TypeName
        && let Some(expected) = &request.expected_name
    {
        prompt = prompt.with_typed_name(expected.clone());
    }
    prompt
}

fn preview_note(preview: &DryRunPreview) -> PreviewNote {
    match preview {
        DryRunPreview::Rehearsed(result) => {
            PreviewNote::Rehearsed(result.message.clone().unwrap_or_default().into())
        }
        DryRunPreview::NoPreviewAvailable { reason } => {
            PreviewNote::Unavailable(reason.clone().into())
        }
    }
}

/// The objects of a bulk command, each with its own risk (a bulk delete's objects differ).
fn items_of(command: &Command, fallback: Option<Risk>) -> Vec<ConfirmItem> {
    let propagation = match command {
        Command::SelectionDelete { propagation, .. } => Some(*propagation),
        _ => None,
    };
    command
        .targets()
        .iter()
        .map(|target| {
            let risk = match propagation {
                Some(propagation) => delete_risk(&target.gvk, propagation),
                None => fallback.unwrap_or(Risk::Medium),
            };
            ConfirmItem::new(object_label(target), risk)
        })
        .collect()
}
