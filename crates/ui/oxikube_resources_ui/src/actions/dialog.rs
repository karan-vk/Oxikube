//! [`DeleteDialog`]: confirm, run and report a delete of one or several objects.
//!
//! The dialog is a modal of the cluster tab's workspace. It plans with
//! [`DeleteFlow::plan`](oxikube_app::DeleteFlow::plan), so what it asks is what the guard will
//! ask; changing the propagation re-plans (a foreground, cascading delete takes the typed name).
//! Cancel sends nothing. Confirm runs the flow on the Tokio bridge and the dialog shows each
//! object's result; a single object that was deleted closes the dialog with a toast instead.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Window,
};
use oxikube_app::{DeleteFlow, DeletePlan, DeleteReport};
use oxikube_domain::command::Propagation;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Asking. Nothing has been sent.
    Confirm,
    /// The deletes are running.
    Running,
    /// Every object has a result.
    Done,
}

/// The delete confirmation. See the [module docs](self).
pub struct DeleteDialog {
    pub(super) flow: DeleteFlow,
    pub(super) plan: DeletePlan,
    pub(super) typed: Entity<InputState>,
    pub(super) stage: Stage,
    pub(super) report: Option<DeleteReport>,
    /// Why nothing ran (the phrase did not match), shown under the field.
    pub(super) error: Option<SharedString>,
    workspace: gpui::WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for DeleteDialog {}

impl Focusable for DeleteDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for DeleteDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the deletes run the dialog stays
    /// (they finish either way, and the results are what the user wants to see).
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != Stage::Running
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != Stage::Running
    }
}

impl DeleteDialog {
    /// A dialog for `plan` (from `flow.plan(targets, Propagation::default())`: the kubectl
    /// default).
    pub fn new(
        flow: DeleteFlow,
        plan: DeletePlan,
        workspace: gpui::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let typed =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type the name to confirm"));
        let subscriptions = vec![cx.subscribe_in(&typed, window, Self::on_typed)];
        Self {
            flow,
            plan,
            typed,
            stage: Stage::Confirm,
            report: None,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the field the user has to type in, or the dialog itself when it has none.
    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plan.phrase().is_some() {
            self.typed.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }

    /// Where the dialog is.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// What the dialog will ask and send.
    pub fn plan(&self) -> &DeletePlan {
        &self.plan
    }

    /// The results, once every object has one.
    pub fn report(&self) -> Option<&DeleteReport> {
        self.report.as_ref()
    }

    /// What has been typed, trimmed.
    pub fn typed_text(&self, cx: &App) -> String {
        self.typed.read(cx).value().trim().to_owned()
    }

    /// Types `text` into the confirmation field, as the user would.
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Chooses how dependents are handled and asks again: a cascading choice raises what has to
    /// be typed.
    pub fn set_propagation(&mut self, propagation: Propagation, cx: &mut Context<Self>) {
        if self.stage != Stage::Confirm || propagation == self.plan.propagation() {
            return;
        }
        let targets: Vec<ResourceRef> = self
            .plan
            .items()
            .iter()
            .map(|item| item.target.clone())
            .collect();
        if let Ok(plan) = self.flow.plan(&targets, propagation) {
            self.plan = plan;
            self.error = None;
            cx.notify();
        }
    }

    /// Whether Delete can be pressed: asking, and the phrase typed when one is needed.
    pub fn can_confirm(&self, cx: &App) -> bool {
        self.stage == Stage::Confirm
            && self
                .plan
                .phrase()
                .is_none_or(|phrase| self.typed_text(cx) == phrase)
    }

    /// Cancels while asking, closes once finished: nothing is sent, so there is nothing to undo.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            cx.emit(DismissEvent);
        }
    }

    /// Deletes. Does nothing while the phrase is not typed.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if !self.can_confirm(cx) {
            return;
        }
        let typed = self.plan.phrase().map(|_| self.typed_text(cx));
        self.stage = Stage::Running;
        self.error = None;
        cx.notify();
        let (flow, plan) = (self.flow.clone(), self.plan.clone());
        let workspace = self.workspace.clone();
        // Detached: the deletes must finish even if the dialog goes away meanwhile (they are
        // already audited as they run); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { flow.run(&plan, typed.as_deref()).await });
            let result = match run.await {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let delivered = this.update(cx, |this, cx| this.finished(&result, cx));
            if delivered.is_err() {
                // The dialog is gone: say how it went where the user still looks.
                let toast = match &result {
                    Ok(report) => summary_toast(report),
                    Err(message) => Toast::error(message.clone()),
                };
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(toast, cx);
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn finished(&mut self, result: &Result<DeleteReport, String>, cx: &mut Context<Self>) {
        match result {
            Ok(report) => {
                self.stage = Stage::Done;
                self.report = Some(report.clone());
                if report.items.len() == 1 && report.failed() == 0 {
                    // A result list for one object that went well is friction: toast and close.
                    self.show_toast(summary_toast(report), cx);
                    cx.emit(DismissEvent);
                } else {
                    cx.notify();
                }
            }
            Err(message) => {
                // Nothing ran (the typed text was wrong): back to asking.
                self.stage = Stage::Confirm;
                self.error = Some(message.clone().into());
                cx.notify();
            }
        }
    }

    fn show_toast(&self, toast: Toast, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            })
            .ok();
    }

    fn on_typed(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.confirm(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

/// The toast for a finished delete.
fn summary_toast(report: &DeleteReport) -> Toast {
    match report.failed() {
        0 => Toast::success(report.summary()),
        _ => Toast::warning(report.summary()),
    }
}
