//! [`DebugDialog`]: the fields of a debug container, their checks, and the run.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, WeakEntity, Window,
};
use oxikube_app::{DebugDefaults, DebugReport, DebugRequest, DebugRunner, split_command};
use oxikube_domain::OxiResult;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugStage {
    /// Asking. Nothing has been sent.
    Editing,
    /// `pod::Debug` is running: the container is being added and started.
    Starting,
}

/// The debug-container dialog. See the [module docs](super).
pub struct DebugDialog {
    pub(super) defaults: DebugDefaults,
    runner: DebugRunner,
    pub(super) image: Entity<InputState>,
    pub(super) command: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    /// The index in `defaults.targets` of the container to share processes with.
    pub(super) target: usize,
    pub(super) stage: DebugStage,
    /// Why the last submit did not go through: a field problem, or the API server's message.
    pub(super) error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for DebugDialog {}

impl Focusable for DebugDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for DebugDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the container starts the dialog stays:
    /// stopping halfway would leave a container nobody opened a terminal in.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage == DebugStage::Editing
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage == DebugStage::Editing
    }
}

/// A one-line input holding `text`.
fn text_field(
    text: &str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<DebugDialog>,
) -> Entity<InputState> {
    let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    state.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    state
}

impl DebugDialog {
    /// A dialog with `defaults` filled in; submitting runs `runner`.
    pub fn new(
        defaults: DebugDefaults,
        runner: DebugRunner,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let image = text_field(&defaults.image, "busybox", window, cx);
        let command = text_field(&defaults.command, "sh", window, cx);
        let name = text_field("", "debugger-xxxxx (generated)", window, cx);
        let subscriptions = [&image, &command, &name]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into();
        Self {
            target: defaults.target,
            defaults,
            runner,
            image,
            command,
            name,
            stage: DebugStage::Editing,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the image field.
    pub fn focus_image(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.image.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Where the dialog is.
    pub fn stage(&self) -> DebugStage {
        self.stage
    }

    /// The message under the fields, if the last submit failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What the dialog started with.
    pub fn defaults(&self) -> &DebugDefaults {
        &self.defaults
    }

    /// The index of the container to share processes with.
    pub fn target_index(&self) -> usize {
        self.target
    }

    /// Chooses the container to share processes with (out of range is ignored).
    pub fn select_target(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.stage == DebugStage::Editing
            && index < self.defaults.targets.len()
            && index != self.target
        {
            self.target = index;
            cx.notify();
        }
    }

    /// Sets the three text fields, as the user typing them would.
    pub fn fill(
        &mut self,
        image: &str,
        command: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, text) in [
            (&self.image, image),
            (&self.command, command),
            (&self.name, name),
        ] {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// The request the fields describe.
    ///
    /// # Errors
    ///
    /// `Validation` saying which field is wrong: an empty or two-word image, an unclosed quote in
    /// the command, a name that is not a DNS label.
    pub fn request(&self, cx: &App) -> OxiResult<DebugRequest> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let mut request = DebugRequest::new(self.defaults.pod.clone(), text(&self.image));
        request.target_container = self
            .defaults
            .targets
            .get(self.target)
            .map(|container| container.name.to_string());
        request.command = split_command(&text(&self.command))?;
        request.name = Some(text(&self.name)).filter(|name| !name.is_empty());
        request.check_fields()?;
        Ok(request)
    }

    /// Cancels while asking: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage == DebugStage::Editing {
            cx.emit(DismissEvent);
        }
    }

    /// Adds the container: checks the fields, then runs `pod::Debug` on the Tokio bridge. A field
    /// problem is shown under the fields and nothing is sent.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.stage != DebugStage::Editing {
            return;
        }
        let request = match self.request(cx) {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error.message().to_owned().into());
                cx.notify();
                return;
            }
        };
        self.stage = DebugStage::Starting;
        self.error = None;
        cx.notify();
        let runner = self.runner.clone();
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is already
        // audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { runner.run(&request).await });
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

    fn finished(&mut self, result: &Result<DebugReport, String>, cx: &mut Context<Self>) {
        match result {
            Ok(report) => {
                // Done: the dialog may close again (`on_before_dismiss` holds it while starting).
                self.stage = DebugStage::Editing;
                self.workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(Toast::success(report.message.clone()), cx);
                    })
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(message) => {
                self.stage = DebugStage::Editing;
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
                if self.error.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => self.submit(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}
