//! [`CanIDialog`]: the fields, the review in flight and its result.

use gpui::{
    App, AppContext as _, ClipboardItem, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, Subscription, Task, Window,
};
use oxikube_app::tools::rbac::{CanIAnswer, CanIRequest, CanIService, CanISubject};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_runtime::{notify_coalesced, spawn_kube};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::memory::SessionMemory;

/// Who the dialog asks as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubjectKind {
    /// The current user.
    #[default]
    Myself,
    /// A user name.
    User,
    /// A group.
    Group,
    /// A ServiceAccount, typed as `namespace/name`.
    ServiceAccount,
}

impl SubjectKind {
    /// Every kind, in the order the dialog lists them.
    pub const ALL: [SubjectKind; 4] = [Self::Myself, Self::User, Self::Group, Self::ServiceAccount];

    /// The label of the kind's button.
    pub fn label(self) -> &'static str {
        match self {
            Self::Myself => "Me",
            Self::User => "User",
            Self::Group => "Group",
            Self::ServiceAccount => "ServiceAccount",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Myself => "",
            Self::User => "user",
            Self::Group => "group",
            Self::ServiceAccount => "serviceaccount",
        }
    }
}

/// Where the dialog's question stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CheckState {
    /// Nothing asked yet (or the fields changed since).
    #[default]
    Idle,
    /// A review is in flight.
    Checking,
    /// The answer.
    Answered(CanIAnswer),
    /// The question could not be asked or answered; the text says why.
    Failed(String),
}

/// What the dialog is built from.
pub struct CanIParts {
    /// The cluster asked.
    pub cluster: ClusterId,
    /// Asks the cluster.
    pub service: CanIService,
    /// The request to prefill (from the selection), with the verb and subject left empty to be
    /// filled from `memory`.
    pub prefill: CanIRequest,
    /// The last verb and subject of this session.
    pub memory: SessionMemory,
}

/// The can-i dialog. See the [module docs](super).
pub struct CanIDialog {
    pub(super) cluster: ClusterId,
    service: CanIService,
    memory: SessionMemory,
    pub(super) verb: Entity<InputState>,
    pub(super) resource: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) subject: Entity<InputState>,
    pub(super) subject_kind: SubjectKind,
    pub(super) state: CheckState,
    /// The review in flight. Replacing it drops the old task, which aborts its request.
    task: Option<Task<()>>,
    pub(super) focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CanIDialog {}

impl Focusable for CanIDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for CanIDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// The typed subject is not lost to a stray click on the scrim.
    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        false
    }

    /// Closing the dialog aborts a review in flight.
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.task = None;
        true
    }
}

impl CanIDialog {
    /// A dialog prefilled from `parts`.
    pub fn new(parts: CanIParts, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let verb = input("get, list, delete, ...", window, cx);
        let resource = input("pods, deployments.apps, pods/exec", window, cx);
        let namespace = input("empty: cluster-scoped or all", window, cx);
        let name = input("optional", window, cx);
        let subject = input("name, or namespace/name", window, cx);
        let (kind, remembered) = parts.memory.subject();
        let prefill = parts.prefill;
        let verb_text = if prefill.verb.is_empty() {
            parts.memory.verb().unwrap_or_else(|| "get".to_owned())
        } else {
            prefill.verb
        };
        verb.update(cx, |input, cx| input.set_value(verb_text, window, cx));
        resource.update(cx, |input, cx| {
            input.set_value(prefill.resource, window, cx)
        });
        namespace.update(cx, |input, cx| {
            input.set_value(prefill.namespace.unwrap_or_default(), window, cx);
        });
        name.update(cx, |input, cx| {
            input.set_value(prefill.name.unwrap_or_default(), window, cx);
        });
        subject.update(cx, |input, cx| input.set_value(remembered, window, cx));
        let mut dialog = Self {
            cluster: parts.cluster,
            service: parts.service,
            memory: parts.memory,
            verb,
            resource,
            namespace,
            name,
            subject,
            subject_kind: kind,
            state: CheckState::Idle,
            task: None,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        let inputs = [
            dialog.verb.clone(),
            dialog.resource.clone(),
            dialog.namespace.clone(),
            dialog.name.clone(),
            dialog.subject.clone(),
        ];
        for input in &inputs {
            let subscription = cx.subscribe_in(input, window, Self::on_input);
            dialog._subscriptions.push(subscription);
        }
        let first = if dialog.resource.read(cx).value().is_empty() {
            &dialog.resource
        } else {
            &dialog.verb
        };
        first.update(cx, |input, cx| input.focus(window, cx));
        dialog
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.ask(window, cx),
            InputEvent::Change => {
                // An old answer no longer matches the fields; a review in flight still does.
                if matches!(self.state, CheckState::Answered(_) | CheckState::Failed(_)) {
                    self.state = CheckState::Idle;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// The question the fields state now.
    pub fn request(&self, cx: &App) -> OxiResult<CanIRequest> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let some = |text: String| (!text.is_empty()).then_some(text);
        let subject_text = text(&self.subject);
        let subject = match self.subject_kind {
            SubjectKind::Myself => CanISubject::Myself,
            kind => CanISubject::parse(kind.word(), &subject_text, None)?,
        };
        Ok(CanIRequest {
            verb: text(&self.verb),
            resource: text(&self.resource),
            namespace: some(text(&self.namespace)),
            name: some(text(&self.name)),
            subject,
        })
    }

    /// Where the question stands.
    pub fn state(&self) -> &CheckState {
        &self.state
    }

    /// Who the dialog asks as.
    pub fn subject_kind(&self) -> SubjectKind {
        self.subject_kind
    }

    /// Chooses who to ask as.
    pub fn set_subject_kind(&mut self, kind: SubjectKind, cx: &mut Context<Self>) {
        if self.subject_kind != kind {
            self.subject_kind = kind;
            self.state = CheckState::Idle;
            cx.notify();
        }
    }

    /// Puts `verb` in the verb field.
    pub fn set_verb(&mut self, verb: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.verb
            .update(cx, |input, cx| input.set_value(verb.to_owned(), window, cx));
    }

    /// Asks the cluster. A review already in flight is aborted first.
    pub fn ask(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let request = match self.request(cx) {
            Ok(request) if request.verb.is_empty() || request.resource.is_empty() => {
                self.state =
                    CheckState::Failed("Enter a verb and a resource to ask about.".to_owned());
                cx.notify();
                return;
            }
            Ok(request) => request,
            Err(error) => {
                self.state = CheckState::Failed(error.message().to_owned());
                cx.notify();
                return;
            }
        };
        let subject_text = self.subject.read(cx).value().trim().to_owned();
        self.memory
            .remember(&request.verb, self.subject_kind, &subject_text);
        self.state = CheckState::Checking;
        let service = self.service.clone();
        let cluster = self.cluster.clone();
        let read = spawn_kube(cx, async move { service.ask(&cluster, &request).await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |this, cx| this.answered(result, cx)).ok();
        }));
        notify_coalesced(cx);
    }

    fn answered(&mut self, result: OxiResult<CanIAnswer>, cx: &mut Context<Self>) {
        self.state = match result {
            Ok(answer) => CheckState::Answered(answer),
            Err(error) => CheckState::Failed(format!(
                "{}: {}",
                error.kind(),
                oxikube_domain::redact::redact(error.message())
            )),
        };
        notify_coalesced(cx);
    }

    /// Copies the answer (question, outcome, reason, method, `kubectl` command) as text.
    pub fn copy_result(&self, cx: &mut Context<Self>) -> bool {
        let CheckState::Answered(answer) = &self.state else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(answer.to_text()));
        true
    }

    /// Closes the dialog.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        cx.emit(DismissEvent);
    }
}

impl CanIDialog {
    /// The verb field.
    pub fn verb_input(&self) -> Entity<InputState> {
        self.verb.clone()
    }

    /// The resource field.
    pub fn resource_input(&self) -> Entity<InputState> {
        self.resource.clone()
    }

    /// The namespace field.
    pub fn namespace_input(&self) -> Entity<InputState> {
        self.namespace.clone()
    }

    /// The subject field.
    pub fn subject_input(&self) -> Entity<InputState> {
        self.subject.clone()
    }
}
