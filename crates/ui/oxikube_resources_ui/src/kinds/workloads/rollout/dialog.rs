//! [`SetImageDialog`]: one field per container, the diff of old to new, and the run.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::{SetImagePlan, SetImageReport, SetImageRunner, WorkloadService};
use oxikube_domain::OxiResult;
use oxikube_domain::build::{ImageEdit, ImageSlot, image_problem};
use oxikube_domain::command::ImageChange;
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetImageStage {
    /// Reading the workload's containers. The form waits.
    Loading,
    /// Asking. Nothing has been sent.
    Editing,
    /// `workload::SetImage` is running.
    Applying,
}

/// One container's row: what it runs now and the field holding what the user wants it to run.
pub struct ContainerField {
    /// The container's name.
    pub name: Arc<str>,
    /// Whether it is an init container.
    pub init: bool,
    /// The image it runs now.
    pub current: Arc<str>,
    pub(super) input: Entity<InputState>,
}

/// The set-image dialog. See the [module docs](super).
pub struct SetImageDialog {
    pub(super) target: ResourceRef,
    runner: SetImageRunner,
    pub(super) plan: Option<SetImagePlan>,
    pub(super) fields: Vec<ContainerField>,
    pub(super) stage: SetImageStage,
    /// Why the last read or run failed: the server's message.
    pub(super) error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    /// The read of the containers; replaced, never cleared from inside.
    _plan_task: Task<()>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SetImageDialog {}

impl Focusable for SetImageDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for SetImageDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the patch runs the dialog stays so
    /// its outcome is not lost.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != SetImageStage::Applying
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != SetImageStage::Applying
    }
}

impl SetImageDialog {
    /// A dialog for `target`. It reads the containers through `service` on the Tokio bridge and
    /// shows the form once that answers; submitting runs `runner`.
    pub fn new(
        target: ResourceRef,
        service: Arc<WorkloadService>,
        runner: SetImageRunner,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let read = {
            let target = target.clone();
            spawn_kube(cx, async move { service.plan_set_image(&target).await })
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
            fields: Vec::new(),
            stage: SetImageStage::Loading,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _plan_task: plan_task,
            subscriptions: Vec::new(),
        }
    }

    /// The containers arrived (or could not be read).
    fn planned(
        &mut self,
        plan: OxiResult<SetImagePlan>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match plan {
            Ok(plan) => {
                let fields: Vec<_> = plan
                    .containers
                    .iter()
                    .map(|slot| self.field_for(slot, window, cx))
                    .collect();
                self.fields = fields;
                self.plan = Some(plan);
                self.stage = SetImageStage::Editing;
                self.focus_first(window, cx);
            }
            Err(error) => {
                // Nothing to edit without the containers: the message says why, Cancel closes.
                self.error = Some(error.message().to_owned().into());
                self.stage = SetImageStage::Editing;
            }
        }
        cx.notify();
    }

    fn field_for(
        &mut self,
        slot: &ImageSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ContainerField {
        let image = slot.image.to_string();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("image");
            state.set_value(image, window, cx);
            state
        });
        self.subscriptions
            .push(cx.subscribe_in(&input, window, Self::on_input));
        ContainerField {
            name: slot.name.clone(),
            init: slot.init,
            current: slot.image.clone(),
            input,
        }
    }

    /// Focuses the first container's field.
    pub fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.fields.first() {
            field.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Where the dialog is.
    pub fn stage(&self) -> SetImageStage {
        self.stage
    }

    /// The workload.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// What the workload had when the dialog read it; `None` until then (or when it could not).
    pub fn plan(&self) -> Option<&SetImagePlan> {
        self.plan.as_ref()
    }

    /// The message under the form, if a read or a run failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The container rows, init containers first.
    pub fn fields(&self) -> &[ContainerField] {
        &self.fields
    }

    /// The text of container `name`'s field; `None` for a container the workload does not have.
    pub fn text(&self, name: &str, cx: &App) -> Option<String> {
        let field = self.fields.iter().find(|field| &*field.name == name)?;
        Some(field.input.read(cx).value().to_string())
    }

    /// Sets container `name`'s field, as the user typing it would.
    pub fn set_text(
        &mut self,
        name: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.fields.iter().find(|field| &*field.name == name) {
            field
                .input
                .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// What is wrong with the field of the container at `index`, in words for the field: empty, or
    /// with a space in it. `None` for a good one.
    pub fn problem(&self, index: usize, cx: &App) -> Option<&'static str> {
        let field = self.fields.get(index)?;
        image_problem(field.input.read(cx).value().as_ref())
    }

    /// Whether any field holds an image the dialog refuses.
    fn has_problem(&self, cx: &App) -> bool {
        self.fields
            .iter()
            .any(|field| image_problem(field.input.read(cx).value().as_ref()).is_some())
    }

    /// The edits the form makes now: every container whose field differs from what it runs, with
    /// its old and new image. This is the diff the dialog shows and the button sends.
    pub fn edits(&self, cx: &App) -> Vec<ImageEdit> {
        self.fields
            .iter()
            .filter_map(|field| {
                let text = field.input.read(cx).value();
                let to = text.trim();
                (image_problem(to).is_none() && to != &*field.current).then(|| ImageEdit {
                    container: field.name.clone(),
                    init: field.init,
                    from: field.current.clone(),
                    to: Arc::from(to),
                })
            })
            .collect()
    }

    /// Whether the button can run now: asking, every field valid, and something changed.
    pub fn can_submit(&self, cx: &App) -> bool {
        self.stage == SetImageStage::Editing && !self.has_problem(cx) && !self.edits(cx).is_empty()
    }

    /// Cancels while not running: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != SetImageStage::Applying {
            cx.emit(DismissEvent);
        }
    }

    /// Sets the images: checks the form, then runs `workload::SetImage` on the Tokio bridge with
    /// only the containers that changed. A field problem shows under the field and nothing is
    /// sent.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            cx.notify();
            return;
        }
        let changes: Vec<ImageChange> = self
            .edits(cx)
            .iter()
            .map(|edit| ImageChange::new(&*edit.container, &*edit.to))
            .collect();
        self.stage = SetImageStage::Applying;
        self.error = None;
        cx.notify();
        let runner = self.runner.clone();
        let target = self.target.clone();
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is already
        // audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { runner.run(&target, changes).await });
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

    fn finished(&mut self, result: &Result<SetImageReport, String>, cx: &mut Context<Self>) {
        self.stage = SetImageStage::Editing;
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
