//! [`ScaleDialog`]: the replica count field, its checks, and the run.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::{ScalePlan, ScaleReport, ScaleRunner, WorkloadService, parse_replicas};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleStage {
    /// Reading the workload's current count. The field waits.
    Loading,
    /// Asking. Nothing has been sent.
    Editing,
    /// `workload::Scale` is running.
    Scaling,
}

/// The scale dialog. See the [module docs](super).
pub struct ScaleDialog {
    pub(super) target: ResourceRef,
    runner: ScaleRunner,
    pub(super) plan: Option<ScalePlan>,
    pub(super) replicas: Entity<InputState>,
    pub(super) stage: ScaleStage,
    /// Why the last read or run failed: the server's message.
    pub(super) error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    /// The read of the current count; replaced, never cleared from inside.
    _plan_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ScaleDialog {}

impl Focusable for ScaleDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for ScaleDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the scale runs the dialog stays so
    /// its outcome is not lost.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != ScaleStage::Scaling
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != ScaleStage::Scaling
    }
}

impl ScaleDialog {
    /// A dialog for `target`. It reads the current count through `service` on the Tokio bridge
    /// and shows the field once that answers; submitting runs `runner`.
    pub fn new(
        target: ResourceRef,
        service: Arc<WorkloadService>,
        runner: ScaleRunner,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let replicas = cx.new(|cx| InputState::new(window, cx).placeholder("replicas"));
        let subscription = cx.subscribe_in(&replicas, window, Self::on_input);
        let read = {
            let target = target.clone();
            spawn_kube(cx, async move { service.plan_scale(&target).await })
        };
        let plan_task = cx.spawn_in(window, async move |this, cx| {
            let plan = read.await.unwrap_or_else(|error| Err(error.into()));
            this.update_in(cx, |this, window, cx| this.planned(plan, window, cx))
                .ok();
        });
        Self {
            target,
            runner,
            plan: None,
            replicas,
            stage: ScaleStage::Loading,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _plan_task: plan_task,
            _subscriptions: vec![subscription],
        }
    }

    /// The current count arrived (or could not be read).
    fn planned(&mut self, plan: OxiResult<ScalePlan>, window: &mut Window, cx: &mut Context<Self>) {
        match plan {
            Ok(plan) => {
                let desired = plan.desired.to_string();
                self.plan = Some(plan);
                self.stage = ScaleStage::Editing;
                self.replicas.update(cx, |input, cx| {
                    input.set_value(desired, window, cx);
                    input.focus(window, cx);
                });
            }
            Err(error) => {
                self.error = Some(error.message().to_owned().into());
                // Still askable: an account that may scale but not read the count can type one.
                self.stage = ScaleStage::Editing;
            }
        }
        cx.notify();
    }

    /// Focuses the replicas field.
    pub fn focus_replicas(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.replicas
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Where the dialog is.
    pub fn stage(&self) -> ScaleStage {
        self.stage
    }

    /// The workload.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// What the workload had when the dialog read it; `None` until then (or when it could not).
    pub fn plan(&self) -> Option<&ScalePlan> {
        self.plan.as_ref()
    }

    /// The message under the field, if a read or a run failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The text of the field.
    pub fn text(&self, cx: &App) -> String {
        self.replicas.read(cx).value().to_string()
    }

    /// The count the field holds, or why it is not one.
    ///
    /// # Errors
    ///
    /// `Validation`, in words for the field: empty, negative, not a whole number, too large.
    pub fn parsed(&self, cx: &App) -> OxiResult<u32> {
        parse_replicas(&self.text(cx))
    }

    /// Sets the field, as the user typing it would.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replicas
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Steps the field by `delta` (the minus and plus buttons), never below zero. A field that is
    /// not a number starts from the workload's desired count.
    pub fn step(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage != ScaleStage::Editing {
            return;
        }
        let from = self
            .parsed(cx)
            .ok()
            .or_else(|| self.plan.as_ref().map(|plan| plan.desired))
            .unwrap_or(0);
        let next = i64::from(from)
            .saturating_add(delta)
            .clamp(0, i64::from(i32::MAX));
        self.set_text(&next.to_string(), window, cx);
    }

    /// Whether the button can run now: asking, with a valid count.
    pub fn can_submit(&self, cx: &App) -> bool {
        self.stage == ScaleStage::Editing && self.parsed(cx).is_ok()
    }

    /// Cancels while not running: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != ScaleStage::Scaling {
            cx.emit(DismissEvent);
        }
    }

    /// Scales: checks the field, then runs `workload::Scale` on the Tokio bridge. A field problem
    /// shows under the field and nothing is sent.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.stage != ScaleStage::Editing {
            return;
        }
        let Ok(replicas) = self.parsed(cx) else {
            // The field shows its own message as the user types; Enter just does nothing.
            cx.notify();
            return;
        };
        self.stage = ScaleStage::Scaling;
        self.error = None;
        cx.notify();
        let runner = self.runner.clone();
        let target = self.target.clone();
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is already
        // audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { runner.run(&target, replicas).await });
            let result = match run.await {
                Ok(result) => result.map_err(|error| error.message().to_owned()),
                Err(error) => Err(error.to_string()),
            };
            let delivered = this.update(cx, |this, cx| this.finished(&result, cx));
            if delivered.is_err() {
                // The dialog is gone: say how it went where the user still looks.
                let toast = match &result {
                    Ok(report) => Toast::success(report.message.clone()),
                    Err(message) => Toast::error(message.clone()),
                };
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finished(&mut self, result: &Result<ScaleReport, String>, cx: &mut Context<Self>) {
        self.stage = ScaleStage::Editing;
        match result {
            Ok(report) => {
                self.workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(Toast::success(report.message.clone()), cx);
                    })
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(message) => {
                self.error = Some(message.clone().into());
                cx.notify();
            }
        }
    }

    fn on_input(
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
            InputEvent::PressEnter { .. } => self.submit(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}
