//! [`StartDialog`]: the modal. It starts the forward through `portforward::Start`
//! ([`ForwardStarter`]) on the Tokio bridge and shows a failure inline with its fix.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::{AnnotationSuggestion, ForwardStarter};
use oxikube_domain::Resource;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_ports::ResourceReader;
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::Workspace;
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::form::{FormErrors, FormInput, StartForm};
use super::ports::{PortChoice, ports_of};
use super::prefs::LastUsed;
use super::problem::{Fix, StartProblem};

/// Where the dialog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Asking. Nothing has been sent.
    Editing,
    /// `portforward::Start` is running.
    Starting,
}

/// The start dialog. See the [module docs](super).
pub struct StartDialog {
    pub(super) form: StartForm,
    pub(super) starter: ForwardStarter,
    pub(super) remote: Entity<InputState>,
    pub(super) local: Entity<InputState>,
    pub(super) bind: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) local_auto: bool,
    pub(super) favourite: bool,
    pub(super) stage: Stage,
    pub(super) problem: Option<StartProblem>,
    /// Whether the target's ports are still being read.
    pub(super) loading_ports: bool,
    /// The forwards the pod's k9s annotations suggest (E15-S07); empty with
    /// `portforward.annotations` off.
    pub(super) suggestions: Vec<AnnotationSuggestion>,
    /// The suggestion whose values are in the fields.
    pub(super) applied: Option<usize>,
    /// Whether the caller named the port: a preselected suggestion then leaves the fields alone.
    explicit_port: bool,
    /// The remote and local texts the dialog itself last put in the fields: when the fields still
    /// hold them, the user has not edited anything, and a late preselect may replace them.
    seeded: (String, String),
    pub(super) workspace: WeakEntity<Workspace>,
    pub(super) focus: FocusHandle,
    /// The read of the target's ports; dropped (and so aborted) with the dialog.
    load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for StartDialog {}

impl Focusable for StartDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for StartDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the start runs the dialog stays
    /// (the request is out and its answer is what the user wants to see).
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage == Stage::Editing
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage == Stage::Editing
    }
}

impl StartDialog {
    /// A dialog for `form`, prefilled with `port` (else the target's first port), starting through
    /// `starter` and reporting in `workspace`.
    pub fn new(
        form: StartForm,
        port: Option<ForwardPort>,
        starter: ForwardStarter,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let last = LastUsed::get(cx);
        let explicit_port = port.is_some();
        let remote_text = match (&port, form.ports.first()) {
            (Some(port), _) => port.to_string(),
            (None, Some(first)) => first.number.to_string(),
            (None, None) => String::new(),
        };
        let default_name = form.default_name(&remote_text);
        let seeded = (remote_text.clone(), String::new());
        let remote = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Port on the target, e.g. 8080")
                .default_value(remote_text)
        });
        let local = cx.new(|cx| InputState::new(window, cx).placeholder("Auto"));
        let bind_text = last
            .bind
            .clone()
            .unwrap_or_else(|| starter.default_bind(&form.target.cluster).to_string());
        let bind = cx.new(|cx| InputState::new(window, cx).default_value(bind_text));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(default_name));
        let subscriptions = [&remote, &local, &bind, &name]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into();
        Self {
            form,
            starter,
            remote,
            local,
            bind,
            name,
            local_auto: last.local_auto,
            favourite: false,
            stage: Stage::Editing,
            problem: None,
            loading_ports: false,
            suggestions: Vec::new(),
            applied: None,
            explicit_port,
            seeded,
            workspace,
            focus: cx.focus_handle(),
            load: None,
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the local port field, where the dialog starts.
    pub fn focus_local(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.local.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Reads the target's ports from the cluster (the dialog was opened without them, from `f` or
    /// the palette) and offers them; the dialog is usable meanwhile.
    pub fn load_ports(
        &mut self,
        reader: Arc<dyn ResourceReader>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ResourceRef {
            gvk,
            namespace,
            name,
            ..
        } = self.form.target.clone();
        self.loading_ports = true;
        let read = spawn_kube(cx, async move {
            reader.get(&gvk, namespace.as_deref(), &name).await
        });
        self.load = Some(cx.spawn_in(window, async move |this, cx| {
            let read = match read.await {
                Ok(Ok(resource)) => Some(resource),
                Ok(Err(error)) => {
                    tracing::debug!(%error, "the target's ports could not be read");
                    None
                }
                Err(error) => {
                    tracing::debug!(%error, "the target's ports could not be read");
                    None
                }
            };
            this.update_in(cx, |this, window, cx| match read {
                Some(resource) => this.set_object(&resource, window, cx),
                None => {
                    // A failed read keeps the ports the dialog was opened with.
                    let kept = this.form.ports.clone();
                    this.set_ports(kept, window, cx);
                }
            })
            .ok();
        }));
    }

    /// Takes what the loaded `object` offers: its ports (unless the dialog has them already) and,
    /// with `portforward.annotations` on, the forwards its k9s annotations suggest.
    pub fn set_object(&mut self, object: &Resource, window: &mut Window, cx: &mut Context<Self>) {
        let ports = if self.form.ports.is_empty() {
            ports_of(object)
        } else {
            self.form.ports.clone()
        };
        self.set_suggestions(self.starter.annotation_suggestions(object), window, cx);
        self.set_ports(ports, window, cx);
    }

    /// Offers `suggestions`. The first `auto` one without a port conflict is preselected: its
    /// values fill the fields unless the caller named a port (nothing starts either way).
    pub fn set_suggestions(
        &mut self,
        suggestions: Vec<AnnotationSuggestion>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suggestions = suggestions;
        self.applied = None;
        let preselect = self
            .suggestions
            .iter()
            .position(|s| s.forward.auto && s.conflict.is_none());
        let untouched = self.remote.read(cx).value().trim() == self.seeded.0
            && self.local.read(cx).value().trim() == self.seeded.1;
        if let Some(index) = preselect
            && !self.explicit_port
            && untouched
        {
            self.use_suggestion(index, window, cx);
        }
        cx.notify();
    }

    /// Fills the remote and the (fixed) local port from suggestion `index`. The container is
    /// shown with the suggestion; a forward is made to the pod, so it is not a field.
    pub fn use_suggestion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(forward) = self.suggestions.get(index).map(|s| s.forward.clone()) else {
            return;
        };
        let bind = self.bind.read(cx).value().trim().to_owned();
        self.fill(
            &forward.remote_port.to_string(),
            &forward.local_port.to_string(),
            &bind,
            window,
            cx,
        );
        self.applied = Some(index);
        self.problem = None;
    }

    /// The suggestions shown.
    pub fn suggestions(&self) -> &[AnnotationSuggestion] {
        &self.suggestions
    }

    /// Offers `ports`; an empty remote port field takes the first.
    pub fn set_ports(
        &mut self,
        ports: Vec<PortChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading_ports = false;
        if let Some(first) = ports.first()
            && self.remote.read(cx).value().trim().is_empty()
        {
            let text = first.number.to_string();
            self.seeded.0.clone_from(&text);
            self.remote
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.form.ports = ports;
        cx.notify();
    }

    /// Shows `problem` under the fields, as a failed start does.
    pub fn show_problem(&mut self, problem: StartProblem, cx: &mut Context<Self>) {
        self.problem = Some(problem);
        cx.notify();
    }

    /// Where the dialog is.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The problem under the fields, if the last start failed.
    pub fn problem(&self) -> Option<&StartProblem> {
        self.problem.as_ref()
    }

    /// The fields as typed.
    pub fn input(&self, cx: &App) -> FormInput {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        FormInput {
            remote_port: text(&self.remote),
            local_auto: self.local_auto,
            local_port: text(&self.local),
            bind: text(&self.bind),
            name: text(&self.name),
            favourite: self.favourite,
        }
    }

    /// What is wrong with the fields now.
    pub fn errors(&self, cx: &App) -> FormErrors {
        self.form.errors(&self.input(cx))
    }

    /// Whether Forward can be pressed: asking, and every field valid.
    pub fn can_submit(&self, cx: &App) -> bool {
        self.stage == Stage::Editing && self.form.can_submit(&self.input(cx))
    }

    /// Types `remote`, a fixed `local` port (empty is Auto) and `bind` into the fields.
    pub fn fill(
        &mut self,
        remote: &str,
        local: &str,
        bind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, text) in [
            (&self.remote, remote),
            (&self.local, local),
            (&self.bind, bind),
        ] {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
        self.seeded = (remote.to_owned(), local.to_owned());
        self.local_auto = local.is_empty();
        cx.notify();
    }

    /// Chooses `port` as the remote port.
    pub fn choose_port(&mut self, port: &PortChoice, window: &mut Window, cx: &mut Context<Self>) {
        let text = port.number.to_string();
        self.seeded.0.clone_from(&text);
        self.remote
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// The Auto checkbox: Auto clears the fixed port.
    pub fn set_auto(&mut self, auto: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.local_auto = auto;
        if auto {
            self.seeded.1.clear();
            self.local
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.problem = None;
        cx.notify();
    }

    /// The favourite checkbox.
    pub fn set_favourite(&mut self, favourite: bool, cx: &mut Context<Self>) {
        self.favourite = favourite;
        cx.notify();
    }

    /// Applies the one-click fix of the problem shown (it does not resubmit).
    pub fn apply_fix(&mut self, fix: Fix, window: &mut Window, cx: &mut Context<Self>) {
        match fix {
            Fix::UseAuto => self.set_auto(true, window, cx),
            Fix::UseLoopback => {
                let loopback = ForwardSpec::DEFAULT_BIND.to_string();
                self.bind
                    .update(cx, |input, cx| input.set_value(loopback, window, cx));
            }
        }
        self.problem = None;
        cx.notify();
    }

    /// Cancels while asking: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage == Stage::Editing {
            cx.emit(DismissEvent);
        }
    }

    fn on_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.problem = None;
                // Typing a port leaves Auto.
                if *input == self.local && !input.read(cx).value().trim().is_empty() {
                    self.local_auto = false;
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.submit(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

impl StartDialog {
    /// The title: `Port-forward pod/web`.
    pub(super) fn title(&self) -> SharedString {
        let target = &self.form.target;
        format!(
            "Port-forward {}/{}",
            target.gvk.kind.to_ascii_lowercase(),
            target.name
        )
        .into()
    }
}
