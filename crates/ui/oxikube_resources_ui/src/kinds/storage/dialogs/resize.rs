//! [`ResizeDialog`]: resize a PersistentVolumeClaim.
//!
//! The dialog is a modal of the cluster tab's workspace. It reads the claim and its StorageClass
//! off the UI thread when it opens, then checks what is typed with the same rule the
//! `pvc::Resize` handler applies ([`ResizeContext::check`]): a size that is not greater than the
//! current one, or a class that does not allow expansion, is refused here with the reason and
//! nothing is sent. Resize sends `pvc::Resize` through the bus; the guard's simple confirmation
//! is the button. Afterwards the dialog shows the claim's conditions, where a pending file
//! system expansion appears.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::{ResizeContext, ResizeFlow, ResizeReport};
use oxikube_domain::Quantity;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::storage::ResizeBlock;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

use super::shared::{settle, show_toast};

/// What the dialog has read.
#[derive(Debug, Clone, PartialEq)]
pub enum Loaded {
    /// Reading the claim and its class.
    Loading,
    /// Read.
    Ready(Box<ResizeContext>),
    /// The read failed (forbidden, gone, not connected), with the reason.
    Failed(SharedString),
}

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeStage {
    /// Asking. Nothing has been sent.
    Asking,
    /// The resize is running.
    Running,
    /// It went through.
    Done,
}

/// The resize dialog. See the module docs.
pub struct ResizeDialog {
    pub(super) flow: ResizeFlow,
    pub(super) target: ResourceRef,
    pub(super) loaded: Loaded,
    pub(super) stage: ResizeStage,
    pub(super) size: Entity<InputState>,
    pub(super) report: Option<ResizeReport>,
    /// Why the last attempt failed (the server's refusal, a changed claim), under the field.
    pub(super) error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ResizeDialog {}

impl Focusable for ResizeDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for ResizeDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; a running resize finishes either way.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != ResizeStage::Running
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != ResizeStage::Running
    }
}

impl ResizeDialog {
    /// A dialog for `target`, reading the claim now.
    pub fn new(
        flow: ResizeFlow,
        target: ResourceRef,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let size =
            cx.new(|cx| InputState::new(window, cx).placeholder("New size, for example 20Gi"));
        let subscriptions = vec![cx.subscribe_in(&size, window, Self::on_typed)];
        let load = {
            let (flow, target) = (flow.clone(), target.clone());
            cx.spawn(async move |this, cx| {
                let read = spawn_kube(cx, async move { flow.open(&target).await }).await;
                let loaded = match settle(read) {
                    Ok(context) => Loaded::Ready(Box::new(context)),
                    Err(reason) => Loaded::Failed(reason.into()),
                };
                this.update(cx, |this, cx| this.loaded(loaded, cx)).ok();
            })
        };
        Self {
            flow,
            target,
            loaded: Loaded::Loading,
            stage: ResizeStage::Asking,
            size,
            report: None,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _load: load,
            _subscriptions: subscriptions,
        }
    }

    fn loaded(&mut self, loaded: Loaded, cx: &mut Context<Self>) {
        self.loaded = loaded;
        cx.notify();
    }

    /// Focuses the size field.
    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.size.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Where the dialog is.
    pub fn stage(&self) -> ResizeStage {
        self.stage
    }

    /// What the dialog has read.
    pub fn loaded_state(&self) -> &Loaded {
        &self.loaded
    }

    /// The claim being resized.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// The result, once the resize went through.
    pub fn report(&self) -> Option<&ResizeReport> {
        self.report.as_ref()
    }

    /// What has been typed, trimmed.
    pub fn typed(&self, cx: &App) -> String {
        self.size.read(cx).value().trim().to_owned()
    }

    /// Types `text` in the size field, as the user would.
    pub fn type_size(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.size
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// The reason the dialog cannot resize at all, whatever is typed: the class forbids
    /// expansion, or the claim has none.
    pub fn blocked(&self) -> Option<ResizeBlock> {
        match &self.loaded {
            Loaded::Ready(context) => context.blocked(),
            _ => None,
        }
    }

    /// The new size, or the reason what is typed is refused. `None` until the claim is read.
    pub fn check(&self, cx: &App) -> Option<Result<Quantity, ResizeBlock>> {
        match &self.loaded {
            Loaded::Ready(context) => Some(context.check(&self.typed(cx))),
            _ => None,
        }
    }

    /// Whether Resize can be pressed: asking, and a valid size typed.
    pub fn can_confirm(&self, cx: &App) -> bool {
        self.stage == ResizeStage::Asking && matches!(self.check(cx), Some(Ok(_)))
    }

    /// Cancels while asking, closes once finished.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != ResizeStage::Running {
            cx.emit(DismissEvent);
        }
    }

    /// Resizes. Does nothing while the size is not valid.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if !self.can_confirm(cx) {
            return;
        }
        let (Loaded::Ready(context), Some(Ok(size))) = (&self.loaded, self.check(cx)) else {
            return;
        };
        let context = (**context).clone();
        self.stage = ResizeStage::Running;
        self.error = None;
        cx.notify();
        let flow = self.flow.clone();
        let workspace = self.workspace.clone();
        let name = format!(
            "{}/{}",
            context.target.namespace().unwrap_or_default(),
            context.target.name
        );
        // Detached: the resize is audited as it runs and must finish even if the dialog goes
        // away meanwhile; the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { flow.run(&context, size).await });
            let result = settle(run.await);
            let delivered = this.update(cx, |this, cx| this.finished(&result, &name, cx));
            if delivered.is_err() {
                let toast = match &result {
                    Ok(report) => Toast::success(format!("Resizing {name} to {}", report.to)),
                    Err(message) => Toast::error(message.clone()),
                };
                show_toast(&workspace, toast, cx);
            }
        })
        .detach();
    }

    fn finished(
        &mut self,
        result: &Result<ResizeReport, String>,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(report) => {
                self.stage = ResizeStage::Done;
                self.report = Some(report.clone());
                if report.conditions.is_empty() {
                    // Nothing more to say: a toast and done.
                    let toast = Toast::success(format!("Resizing {name} to {}", report.to));
                    show_toast(&self.workspace, toast, cx);
                    cx.emit(DismissEvent);
                } else {
                    cx.notify();
                }
            }
            Err(message) => {
                self.stage = ResizeStage::Asking;
                self.error = Some(message.clone().into());
                cx.notify();
            }
        }
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
