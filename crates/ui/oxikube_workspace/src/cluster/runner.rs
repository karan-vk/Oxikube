//! [`ClusterCommandRunner`]: dispatches a [`Command`] for the UI and shows what came back.
//!
//! Buttons, menus and the tab context menu never call the guard or a port: they build a
//! [`Command`] and call [`ClusterCommandRunner::run`]. The runner dispatches it on the bus as
//! [`Initiator::Ui`], off the UI thread (`oxikube_runtime::spawn_kube`, abort-on-drop), and turns
//! the answer into UI.
//!
//! An immediate command (the cluster tab commands, `namespace::Select`: UI and session state in
//! memory, never guarded) runs in the caller's update instead (E05-P600):
//! [`CommandBus::dispatch_now`], then the cluster tabs' queue is applied and the session updates
//! it made are echoed to the views ([`SessionEcho`]), so the frame drawn right after the input
//! shows its effect. Its rest (a state store write) runs off the UI thread; a failure is a toast.
//!
//! The answers:
//!
//! * completed: a toast with the handler's message, if it had one, and an "Open" button when the
//!   handler named an object to go to next (`cronjob::Trigger`: the Job it created; the button
//!   dispatches `resource::Open` for it);
//! * needs confirmation: a [`DialogModal`] with the guard's one-line summary; confirming
//!   dispatches the same command again with the answer, cancelling declines it (audited);
//! * refused: a toast naming the cluster ([`denial_toast`]). A read-only refusal also pulses the
//!   status bar badge once. The refusal came from the guard; the UI only reports it.

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, WeakEntity, Window};
use oxikube_app::{
    ClusterSessionManager, CommandBus, Confirmation, ConfirmationRequest, ConfirmationToken,
    DispatchContext, DispatchError, Outcome,
};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;
use oxikube_runtime::spawn_kube;

use super::echo::SessionEcho;
use super::status::ClusterStatusItem;
use crate::cluster_tab::apply_queued;
use crate::modal::DialogModal;
use crate::toast::{Toast, ToastAction};
use crate::workspace::Workspace;

/// The toast for a dispatch that did not run.
///
/// A read-only refusal is a warning that names the cluster and says how to proceed; it carries a
/// key per cluster so repeated refusals update one toast instead of stacking. Everything else is
/// an error toast with the error's own message (already redacted by its producer).
pub fn denial_toast(error: &DispatchError) -> Toast {
    match error {
        DispatchError::ReadOnly { cluster, context } => Toast::warning(format!(
            "{context} is read-only. Turn off read-only mode to make changes."
        ))
        .key(format!("read-only:{cluster}")),
        other => Toast::error(other.to_string()),
    }
}

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
        Command::RoleCreate { .. } => ("Create the role?", "Create", false),
        Command::RoleEdit { .. } => ("Save the role?", "Save", false),
        Command::RoleDelete { .. } => ("Delete the role?", "Delete", true),
        _ => ("Are you sure?", "Turn off", true),
    }
}

/// Dispatches commands from views. See the module docs.
#[derive(Clone)]
pub struct ClusterCommandRunner {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    who: Arc<str>,
    workspace: WeakEntity<Workspace>,
    status: Option<WeakEntity<ClusterStatusItem>>,
}

impl ClusterCommandRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name for the audit log), showing
    /// toasts and dialogs in `workspace`. `sessions` are the sessions its immediate commands
    /// change (their updates are echoed to the views at once).
    pub fn new(
        bus: CommandBus,
        sessions: ClusterSessionManager,
        who: impl Into<Arc<str>>,
        workspace: &Entity<Workspace>,
    ) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
            workspace: workspace.downgrade(),
            status: None,
        }
    }

    /// Pulses `item` when a mutation is refused on its cluster.
    #[must_use]
    pub fn with_status_item(mut self, item: &Entity<ClusterStatusItem>) -> Self {
        self.status = Some(item.downgrade());
        self
    }

    /// Dispatches `command` as the UI.
    pub fn run(&self, command: Command, window: &mut Window, cx: &mut App) {
        self.dispatch(command, None, window, cx);
    }

    fn dispatch(
        &self,
        command: Command,
        confirmation: Option<Confirmation>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut context = DispatchContext::new(Initiator::Ui, self.who.clone());
        if confirmation.is_none() && self.bus.runs_now(command.id()) {
            self.dispatch_now(command, context, window, cx);
            return;
        }
        context.confirmation = confirmation;
        let runner = self.clone();
        let bus = self.bus.clone();
        let sent = command.clone();
        window
            .spawn(cx, async move |cx| {
                let result = spawn_kube(cx, async move { bus.dispatch(command, context).await })
                    .await
                    .map_err(|err| DispatchError::Handler(err.into()));
                // A closed window ends the update with an error: nothing left to show.
                cx.update(|window, cx| runner.finish(sent, result.and_then(|r| r), window, cx))
                    .ok();
            })
            .detach();
    }

    /// Runs an immediate command in this update (see the module docs).
    fn dispatch_now(
        &self,
        command: Command,
        context: DispatchContext,
        window: &mut Window,
        cx: &mut App,
    ) {
        let echo = SessionEcho::begin(&self.sessions);
        let result = self.bus.dispatch_now(command.clone(), context);
        apply_queued(window, cx);
        echo.finish(cx);
        let outcome = result.map(|done| {
            if let Some(rest) = done.rest {
                let runner = self.clone();
                // Detached: the rest completes the command the user asked for, and only a
                // failure has something to show.
                window
                    .spawn(cx, async move |cx| {
                        let result = spawn_kube(cx, rest)
                            .await
                            .map_err(|err| DispatchError::Handler(err.into()))
                            .and_then(|r| r.map_err(DispatchError::Handler));
                        if let Err(error) = result {
                            cx.update(|_, cx| runner.toast(denial_toast(&error), cx))
                                .ok();
                        }
                    })
                    .detach();
            }
            Outcome::Completed(done.output)
        });
        self.finish(command, outcome, window, cx);
    }

    fn finish(
        &self,
        command: Command,
        result: Result<Outcome, DispatchError>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match result {
            Ok(Outcome::Completed(output)) => {
                let open = output.open_target();
                if let Some(message) = output.message {
                    self.toast(self.success_toast(message, open), cx);
                }
            }
            Ok(Outcome::NeedsConfirmation(request)) => self.confirm(command, request, window, cx),
            Err(error) => {
                if let Some(cluster) = error.read_only_cluster().cloned()
                    && let Some(status) = self.status.as_ref().and_then(WeakEntity::upgrade)
                {
                    status.update(cx, |item, cx| item.pulse(&cluster, cx));
                }
                self.toast(denial_toast(&error), cx);
            }
        }
    }

    /// The toast for a command that ran: `message`, with an "Open <Kind>" button when `open` names
    /// the object to go to next.
    fn success_toast(&self, message: String, open: Option<ResourceRef>) -> Toast {
        let toast = Toast::success(message);
        let Some(target) = open else {
            return toast;
        };
        let runner = self.clone();
        let label = format!("Open {}", target.gvk.kind);
        toast.action(ToastAction::new(label, move |window, cx| {
            let command = Command::ResourceOpen {
                target: target.clone(),
            };
            runner.run(command, window, cx);
        }))
    }

    fn toast(&self, toast: Toast, cx: &mut App) {
        self.workspace
            .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
            .ok();
    }

    fn confirm(
        &self,
        command: Command,
        request: ConfirmationRequest,
        window: &mut Window,
        cx: &mut App,
    ) {
        let token = request.token;
        let (title, confirm_label, destructive) = confirm_copy(&command);
        // The `TypeName` tier (a force delete): the dialog confirms only once the name is typed,
        // and the answer carries it for the guard to check again.
        let typed = request
            .expected_name
            .clone()
            .filter(|_| request.tier == ConfirmTier::TypeName);
        let (on_confirm, on_decline) = (self.clone(), self.clone());
        self.workspace
            .update(cx, |workspace, cx| {
                let dialog = cx.new(|cx| {
                    let modal = DialogModal::new(title, cx)
                        .message(request.summary)
                        .confirm_label(confirm_label);
                    let modal = if destructive {
                        modal.destructive()
                    } else {
                        modal
                    };
                    let modal = match &typed {
                        Some(name) => modal.require_typed(name.clone(), window, cx),
                        None => modal,
                    };
                    modal
                        .on_confirm(move |window, cx| {
                            let answer = match &typed {
                                Some(name) => Confirmation::typed(token, name.clone()),
                                None => Confirmation::simple(token),
                            };
                            on_confirm.dispatch(command.clone(), Some(answer), window, cx);
                        })
                        .on_cancel(move |_, cx| on_decline.decline(token, cx))
                });
                workspace.show_modal(dialog.clone(), window, cx);
                dialog.update(cx, |dialog, cx| dialog.focus_typed(window, cx));
            })
            .ok();
    }

    /// Tells the guard the user said no, so the refusal is audited.
    fn decline(&self, token: ConfirmationToken, cx: &mut App) {
        let bus = self.bus.clone();
        // Detached: a one-shot whose result only matters to the audit log.
        cx.spawn(async move |cx| {
            let declined = spawn_kube(cx, async move { bus.decline(token).await }).await;
            if !matches!(declined, Ok(Ok(()))) {
                tracing::warn!("could not record a declined confirmation");
            }
        })
        .detach();
    }
}
