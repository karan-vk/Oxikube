//! [`ClusterCommandRunner`]: dispatches a [`Command`] for the UI and shows what came back.
//!
//! Buttons, menus and the tab context menu never call the guard or a port: they build a
//! [`Command`] and call [`ClusterCommandRunner::run`]. The runner dispatches it on the bus as
//! [`Initiator::Ui`], off the UI thread (`oxikube_runtime::spawn_kube`, abort-on-drop), and turns
//! the answer into UI:
//!
//! * completed: a toast with the handler's message, if it had one;
//! * needs confirmation: a [`DialogModal`] with the guard's one-line summary; confirming
//!   dispatches the same command again with the answer, cancelling declines it (audited);
//! * refused: a toast naming the cluster ([`denial_toast`]). A read-only refusal also pulses the
//!   status bar badge once. The refusal came from the guard; the UI only reports it.

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, WeakEntity, Window};
use oxikube_app::{
    CommandBus, Confirmation, ConfirmationRequest, ConfirmationToken, DispatchContext,
    DispatchError, Outcome,
};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;

use super::status::ClusterStatusItem;
use crate::modal::DialogModal;
use crate::toast::Toast;
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

/// Dispatches commands from views. See the [module docs](self).
#[derive(Clone)]
pub struct ClusterCommandRunner {
    bus: CommandBus,
    who: Arc<str>,
    workspace: WeakEntity<Workspace>,
    status: Option<WeakEntity<ClusterStatusItem>>,
}

impl ClusterCommandRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name for the audit log), showing
    /// toasts and dialogs in `workspace`.
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>, workspace: &Entity<Workspace>) -> Self {
        Self {
            bus,
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

    fn finish(
        &self,
        command: Command,
        result: Result<Outcome, DispatchError>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match result {
            Ok(Outcome::Completed(output)) => {
                if let Some(message) = output.message {
                    self.toast(Toast::success(message), cx);
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
        let (on_confirm, on_decline) = (self.clone(), self.clone());
        self.workspace
            .update(cx, |workspace, cx| {
                // The posture commands lower protection (a destructive "Turn off"); a debug
                // container (E09-S10) is an addition that cannot be undone, with its own wording.
                let adds = matches!(command, Command::PodDebug { .. });
                let (title, confirm) = if adds {
                    ("Add a debug container?", "Add debug container")
                } else {
                    ("Are you sure?", "Turn off")
                };
                let dialog = cx.new(|cx| {
                    let modal = DialogModal::new(title, cx)
                        .message(request.summary)
                        .confirm_label(confirm);
                    let modal = if adds { modal } else { modal.destructive() };
                    modal
                        .on_confirm(move |window, cx| {
                            on_confirm.dispatch(
                                command.clone(),
                                Some(Confirmation::simple(token)),
                                window,
                                cx,
                            );
                        })
                        .on_cancel(move |_, cx| on_decline.decline(token, cx))
                });
                workspace.show_modal(dialog, window, cx);
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
