//! [`DrainDialog`]: the options, the plan and the typed name. Starting, following, cancelling and
//! uncordoning are in `run`.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::nodes::{DrainContext, DrainFlow};
use oxikube_domain::drain::{DEFAULT_TIMEOUT_SECS, DrainBoard, DrainFlags, DrainPlan};
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::rows::{DrainLine, plan_lines};

/// What the dialog has read.
#[derive(Debug, Clone, PartialEq)]
pub enum DrainLoaded {
    /// Reading the node and its pods.
    Loading,
    /// Read.
    Ready(Box<DrainContext>),
    /// The read failed, with the reason.
    Failed(SharedString),
}

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainStage {
    /// Choosing the flags and typing the name. Nothing has been sent.
    Asking,
    /// The drain runs; the rows follow it.
    Draining,
    /// It ended (finished, failed or cancelled); the rows say how.
    Done,
}

/// The drain dialog. See the [module docs](super).
pub struct DrainDialog {
    pub(super) flow: DrainFlow,
    pub(super) target: ResourceRef,
    pub(super) loaded: DrainLoaded,
    pub(super) flags: DrainFlags,
    /// The plan for `flags`, made when the pods arrive and on every flag change.
    pub(super) plan: Option<DrainPlan>,
    /// What the list shows: the plan's lines, then the board's.
    pub(super) lines: Vec<DrainLine>,
    pub(super) board: DrainBoard,
    pub(super) stage: DrainStage,
    pub(super) grace: Entity<InputState>,
    pub(super) timeout: Entity<InputState>,
    pub(super) typed: Entity<InputState>,
    /// Why the last start failed, or how the drain ended.
    pub(super) error: Option<SharedString>,
    pub(super) message: Option<SharedString>,
    pub(super) workspace: WeakEntity<Workspace>,
    pub(super) dispatcher: Rc<dyn CommandDispatcher>,
    pub(super) focus: FocusHandle,
    /// The drain: dropping it cancels the dispatch (and so the drain). Never cleared from inside.
    pub(super) run: Option<Task<()>>,
    /// The follower of the drain's steps.
    pub(super) follow: Option<Task<()>>,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for DrainDialog {}

impl Focusable for DrainDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for DrainDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// While the drain runs the dialog stays: only "Cancel drain" stops it.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != DrainStage::Draining
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage == DrainStage::Asking
    }
}

impl DrainDialog {
    /// A dialog for the node `target`, reading it and its pods now through `flow`.
    pub fn new(
        flow: DrainFlow,
        target: ResourceRef,
        workspace: WeakEntity<Workspace>,
        dispatcher: Rc<dyn CommandDispatcher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let flags = DrainFlags::default();
        let grace = cx.new(|cx| InputState::new(window, cx).placeholder("pod's own"));
        let timeout = cx.new(|cx| InputState::new(window, cx).placeholder("seconds"));
        timeout.update(cx, |input, cx| {
            input.set_value(DEFAULT_TIMEOUT_SECS.to_string(), window, cx);
        });
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder(target.name.to_string()));
        let subscriptions = vec![
            cx.subscribe_in(&grace, window, Self::on_number),
            cx.subscribe_in(&timeout, window, Self::on_number),
            cx.subscribe_in(&typed, window, Self::on_typed),
        ];
        let load = {
            let flow = flow.clone();
            let target = target.clone();
            cx.spawn(async move |this, cx| {
                let read = spawn_kube(cx, async move { flow.open(&target).await }).await;
                let loaded = match read {
                    Ok(Ok(context)) => DrainLoaded::Ready(Box::new(context)),
                    Ok(Err(error)) => DrainLoaded::Failed(error.message().to_owned().into()),
                    Err(error) => DrainLoaded::Failed(error.to_string().into()),
                };
                this.update(cx, |this, cx| this.loaded(loaded, cx)).ok();
            })
        };
        Self {
            flow,
            target,
            loaded: DrainLoaded::Loading,
            flags,
            plan: None,
            lines: Vec::new(),
            board: DrainBoard::new(),
            stage: DrainStage::Asking,
            grace,
            timeout,
            typed,
            error: None,
            message: None,
            workspace,
            dispatcher,
            focus: cx.focus_handle(),
            run: None,
            follow: None,
            _load: load,
            _subscriptions: subscriptions,
        }
    }

    fn loaded(&mut self, loaded: DrainLoaded, cx: &mut Context<Self>) {
        self.loaded = loaded;
        self.replan();
        cx.notify();
    }

    /// The plan and its lines for the current flags (off the render path).
    fn replan(&mut self) {
        if self.stage != DrainStage::Asking {
            return;
        }
        self.plan = match &self.loaded {
            DrainLoaded::Ready(context) => Some(context.plan(&self.flags)),
            _ => None,
        };
        self.lines = self.plan.as_ref().map(plan_lines).unwrap_or_default();
    }

    /// Focuses the name field.
    pub fn focus_name(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.typed.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The node.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// What the dialog has read.
    pub fn loaded_state(&self) -> &DrainLoaded {
        &self.loaded
    }

    /// Where the dialog is.
    pub fn stage(&self) -> DrainStage {
        self.stage
    }

    /// The flags as chosen (the numbers as last typed validly).
    pub fn flags(&self) -> &DrainFlags {
        &self.flags
    }

    /// The plan for the chosen flags, once the pods are read.
    pub fn plan(&self) -> Option<&DrainPlan> {
        self.plan.as_ref()
    }

    /// The running (or ended) drain, one row per pod.
    pub fn board(&self) -> &DrainBoard {
        &self.board
    }

    /// Whether a drain of this node runs now, from this dialog or any other caller.
    pub fn drain_running(&self) -> bool {
        self.flow.is_running(&self.target)
    }

    /// The lines the list shows now.
    pub fn lines(&self) -> &[DrainLine] {
        &self.lines
    }

    /// The last failure, or `None`.
    pub fn error_text(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// How the drain ended, once it did.
    pub fn message_text(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The `kubectl drain` line the choices stand for.
    pub fn kubectl_line(&self) -> String {
        self.flags.kubectl_command(&self.target.name)
    }

    /// Sets one of the three switches (by `kubectl` flag) and plans again.
    pub fn set_flag(&mut self, flag: DrainSwitch, on: bool, cx: &mut Context<Self>) {
        if self.stage != DrainStage::Asking {
            return;
        }
        match flag {
            DrainSwitch::IgnoreDaemonSets => self.flags.ignore_daemonsets = on,
            DrainSwitch::DeleteEmptyDirData => self.flags.delete_emptydir_data = on,
            DrainSwitch::Force => self.flags.force = on,
        }
        self.replan();
        cx.notify();
    }

    /// Types `text` into the name field, as the user would.
    pub fn type_name(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.typed
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Why Drain cannot be pressed now, or `None` when it can.
    pub fn start_blocker(&self, cx: &App) -> Option<String> {
        if self.stage != DrainStage::Asking {
            return Some("The drain has started.".into());
        }
        let plan = self.plan.as_ref()?;
        if let Some(first) = plan.blocked.first() {
            return Some(format!(
                "Pod {}: {}. Turn on {} to drain past it.",
                first.pod,
                first.reason.summary(),
                first.reason.flag()
            ));
        }
        if let Err(error) = self.numbers(cx) {
            return Some(error);
        }
        if self.typed.read(cx).value().trim() != &*self.target.name {
            return Some(format!("Type {} to confirm.", self.target.name));
        }
        None
    }

    /// Whether Drain can be pressed: read, nothing blocks, the numbers are numbers, the name typed.
    pub fn can_start(&self, cx: &App) -> bool {
        self.plan.is_some() && self.start_blocker(cx).is_none()
    }

    /// The grace period and timeout fields, read.
    pub(super) fn numbers(&self, cx: &App) -> Result<(Option<u32>, u32), String> {
        let grace = self.grace.read(cx).value().trim().to_owned();
        let grace = if grace.is_empty() || grace == "-1" {
            None
        } else {
            Some(
                grace
                    .parse::<u32>()
                    .map_err(|_| "The grace period is a number of seconds.".to_owned())?,
            )
        };
        let timeout = self
            .timeout
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .map_err(|_| "The timeout is a number of seconds.".to_owned())?;
        let flags = DrainFlags {
            grace_period_secs: grace,
            timeout_secs: timeout,
            ..self.flags.clone()
        };
        flags.validate().map_err(|e| e.message().to_owned())?;
        Ok((grace, timeout))
    }

    /// Closes the dialog unless the drain runs.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.stage != DrainStage::Draining {
            cx.emit(DismissEvent);
        }
    }

    fn on_number(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            // The kubectl line follows the numbers as soon as they are numbers.
            if let Ok((grace, timeout)) = self.numbers(cx) {
                self.flags.grace_period_secs = grace;
                self.flags.timeout_secs = timeout;
            }
            cx.notify();
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
            InputEvent::PressEnter { .. } => self.start(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

/// The three switches of the dialog, by their `kubectl drain` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainSwitch {
    /// `--ignore-daemonsets`.
    IgnoreDaemonSets,
    /// `--delete-emptydir-data`.
    DeleteEmptyDirData,
    /// `--force`.
    Force,
}
