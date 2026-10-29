//! [`CopyDialog`]: the review of a copy (the target's dry run), the choice about an object that
//! exists, the confirmation, and the run.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::{
    CopyChoice, CopyError, CopyOutcome, CopyPreview, CopyRequest, CopyService, ReviewAnswer,
};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

use super::lines::rename_suggestion;

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyStage {
    /// The target's dry run is running (or the target is connecting).
    Reviewing,
    /// The review is shown, or why there is none. Nothing has been written.
    Ready,
    /// `resource::CopyTo` is running.
    Copying,
}

/// What to do about an object that exists at the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Server-side apply over it.
    Overwrite,
    /// Leave it.
    Skip,
    /// Review the copy again under another name.
    Rename,
}

/// The copy dialog. See the [module docs](super).
pub struct CopyDialog {
    service: CopyService,
    pub(super) source: ResourceRef,
    cluster: ClusterId,
    pub(super) cluster_name: SharedString,
    pub(super) namespace: Option<String>,
    pub(super) name: Entity<InputState>,
    pub(super) typed: Entity<InputState>,
    pub(super) stage: CopyStage,
    pub(super) preview: Option<CopyPreview>,
    pub(super) error: Option<CopyError>,
    pub(super) conflict: Option<Conflict>,
    pub(super) force: bool,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    /// The dry run; replaced by the next review (which cancels it), ended with the dialog.
    review_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CopyDialog {}

impl Focusable for CopyDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for CopyDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel until the copy is sent; while it runs the dialog stays so
    /// its outcome is not lost.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != CopyStage::Copying
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != CopyStage::Copying
    }
}

impl CopyDialog {
    /// A dialog copying `source` to `namespace` of `cluster` (shown as `cluster_name`) through
    /// `service`. It starts the target's dry run at once.
    #[expect(
        clippy::too_many_arguments,
        reason = "the flow hands over what the two pickers decided"
    )]
    pub fn new(
        source: ResourceRef,
        cluster: ClusterId,
        cluster_name: impl Into<SharedString>,
        namespace: Option<String>,
        service: CopyService,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("name of the copy"));
        name.update(cx, |input, cx| {
            input.set_value(source.name.to_string(), window, cx);
        });
        let typed =
            cx.new(|cx| InputState::new(window, cx).placeholder("type the name to confirm"));
        let subscriptions = vec![
            cx.subscribe_in(&name, window, Self::on_name),
            cx.subscribe_in(&typed, window, Self::on_typed),
        ];
        let mut dialog = Self {
            service,
            source,
            cluster,
            cluster_name: cluster_name.into(),
            namespace,
            name,
            typed,
            stage: CopyStage::Reviewing,
            preview: None,
            error: None,
            conflict: None,
            force: false,
            workspace,
            focus: cx.focus_handle(),
            review_task: None,
            _subscriptions: subscriptions,
        };
        dialog.review(cx);
        dialog
    }

    /// Focuses the dialog, so Escape and Enter work at once.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// Where the dialog is.
    pub fn stage(&self) -> CopyStage {
        self.stage
    }

    /// The review, once the dry run answered.
    pub fn preview(&self) -> Option<&CopyPreview> {
        self.preview.as_ref()
    }

    /// Why there is no review, or why the last copy failed.
    pub fn error(&self) -> Option<&CopyError> {
        self.error.as_ref()
    }

    /// The choice about an existing object, once made.
    pub fn conflict(&self) -> Option<Conflict> {
        self.conflict
    }

    /// The name typed for the copy.
    pub fn name_text(&self, cx: &App) -> String {
        let text = self.name.read(cx).value().trim().to_owned();
        if text.is_empty() {
            self.source.name.to_string()
        } else {
            text
        }
    }

    /// Types `name` into the name field, as the user would (the review is re-run on Enter or
    /// with [`review`](Self::review)).
    pub fn set_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
    }

    /// Types the confirmation name, as the user would.
    pub fn set_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// The copy the dialog describes now.
    fn request(&self, cx: &App) -> CopyRequest {
        CopyRequest::to(
            self.source.clone(),
            self.cluster.clone(),
            self.namespace.as_deref(),
        )
        .renamed(&self.name_text(cx))
    }

    /// Runs the target's dry run for the current name: connects the target when it is not, reads
    /// the source, shows the diff. Cancels the review before it.
    pub fn review(&mut self, cx: &mut Context<Self>) {
        let request = self.request(cx);
        self.stage = CopyStage::Reviewing;
        self.preview = None;
        self.error = None;
        self.conflict = None;
        self.force = false;
        cx.notify();
        let service = self.service.clone();
        let run = spawn_kube(cx, async move { service.preview(request).await });
        self.review_task = Some(cx.spawn(async move |this, cx| {
            let result = match run.await {
                Ok(result) => result,
                Err(error) => Err(CopyError::Failed(error.to_string())),
            };
            this.update(cx, |this, cx| this.reviewed(result, cx)).ok();
        }));
    }

    fn reviewed(&mut self, result: Result<CopyPreview, CopyError>, cx: &mut Context<Self>) {
        self.stage = CopyStage::Ready;
        match result {
            Ok(preview) => self.preview = Some(preview),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    /// Chooses what to do about the object that exists. Rename suggests `<name>-copy` and
    /// reviews the copy under it; the others wait for the button.
    pub fn choose(&mut self, conflict: Conflict, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage != CopyStage::Ready || !self.preview.as_ref().is_some_and(|p| p.exists()) {
            return;
        }
        if conflict == Conflict::Rename {
            let suggestion = rename_suggestion(&self.name_text(cx));
            self.set_name(&suggestion, window, cx);
            self.review(cx);
            return;
        }
        self.conflict = Some(conflict);
        cx.notify();
    }

    /// Ticks or clears the extra confirmation that takes over fields other managers own.
    pub fn set_force(&mut self, force: bool, cx: &mut Context<Self>) {
        self.force = force;
        cx.notify();
    }

    /// The name the target's guard will ask for, when the copy is a type-the-name tier.
    pub fn expected_name(&self) -> Option<&str> {
        self.preview
            .as_ref()
            .filter(|preview| preview.tier == ConfirmTier::TypeName)
            .map(|preview| &*preview.target.name)
    }

    /// Whether the button can run now.
    pub fn can_copy(&self, cx: &App) -> bool {
        let Some(preview) = self
            .preview
            .as_ref()
            .filter(|_| self.stage == CopyStage::Ready)
        else {
            return false;
        };
        let decided = match (preview.exists(), self.conflict) {
            (false, _) => true,
            (true, Some(Conflict::Skip)) => true,
            (true, Some(Conflict::Overwrite)) => preview.conflicts.is_empty() || self.force,
            (true, Some(Conflict::Rename) | None) => false,
        };
        let confirmed = self
            .expected_name()
            .is_none_or(|expected| self.typed.read(cx).value().trim() == expected);
        decided && confirmed
    }

    /// Cancels while nothing is being written: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != CopyStage::Copying {
            cx.emit(DismissEvent);
        }
    }

    /// Copies (or skips): runs `resource::CopyTo` on the Tokio bridge. Does nothing when
    /// [`can_copy`](Self::can_copy) is false.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_copy(cx) {
            return;
        }
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let choice = if preview.exists() {
            match self.conflict {
                Some(Conflict::Skip) => Some(CopyChoice::Skip),
                _ => Some(CopyChoice::Overwrite { force: self.force }),
            }
        } else {
            None
        };
        let confirm = match self.expected_name() {
            Some(_) => ReviewAnswer::typed(self.typed.read(cx).value().trim()),
            None => ReviewAnswer::click(),
        };
        self.stage = CopyStage::Copying;
        self.error = None;
        cx.notify();
        let service = self.service.clone();
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is audited
        // as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(
                cx,
                async move { service.copy(&preview, choice, confirm).await },
            );
            let result = match run.await {
                Ok(result) => result,
                Err(error) => Err(CopyError::Failed(error.to_string())),
            };
            let delivered = this.update(cx, |this, cx| this.finished(&result, cx));
            if delivered.is_err() {
                // The dialog is gone: say how it went where the user still looks.
                let toast = match &result {
                    Ok(outcome) => outcome_toast(outcome),
                    Err(error) => Toast::error(error.to_string()),
                };
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finished(&mut self, result: &Result<CopyOutcome, CopyError>, cx: &mut Context<Self>) {
        self.stage = CopyStage::Ready;
        match result {
            Ok(outcome) => {
                let toast = outcome_toast(outcome);
                self.workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(error) => {
                // The server named fields other managers own: the force option appears.
                if let (CopyError::Conflicts(conflicts), Some(preview)) =
                    (error, self.preview.as_mut())
                {
                    preview.conflicts.clone_from(conflicts);
                }
                self.error = Some(error.clone());
                cx.notify();
            }
        }
    }

    /// Creates the namespace the target lacks (its own guarded `namespace::Create`), then
    /// reviews the copy again. Does nothing unless the review failed for that.
    pub fn create_namespace(&mut self, cx: &mut Context<Self>) {
        let Some(CopyError::NamespaceMissing(namespace)) = self.error.clone() else {
            return;
        };
        self.stage = CopyStage::Reviewing;
        self.error = None;
        cx.notify();
        let (service, cluster) = (self.service.clone(), self.cluster.clone());
        let run = spawn_kube(cx, async move {
            service.create_namespace(&cluster, &namespace).await
        });
        self.review_task = Some(cx.spawn(async move |this, cx| {
            let result = match run.await {
                Ok(result) => result,
                Err(error) => Err(CopyError::Failed(error.to_string())),
            };
            this.update(cx, |this, cx| match result {
                Ok(message) => {
                    this.workspace
                        .update(cx, |workspace, cx| {
                            workspace.show_toast(Toast::success(message), cx);
                        })
                        .ok();
                    this.review(cx);
                }
                Err(error) => this.reviewed(Err(error), cx),
            })
            .ok();
        }));
    }

    fn on_name(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter { .. } = event {
            self.review(cx);
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
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => self.submit(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

fn outcome_toast(outcome: &CopyOutcome) -> Toast {
    match outcome {
        CopyOutcome::Copied { message, .. } => Toast::success(message.clone()),
        CopyOutcome::Skipped { target } => Toast::info(format!(
            "Skipped {}: it exists on the target",
            oxikube_app::object_label(target)
        )),
    }
}
