//! [`BindingDialog`]: the RoleBinding / ClusterRoleBinding editor.
//!
//! A role picker that offers only what the binding may name, and rows of subjects (a kind select,
//! a name with autocomplete, a ServiceAccount's namespace with the identity it authenticates as).
//! It builds the object as JSON ([`BindingDraft::manifest`]) and sends it as `binding::Create` or
//! `binding::Edit`: the guard confirms it (risk medium), the handler dry-runs it and writes it.
//! The dialog never calls a port that writes. "Edit as YAML" hands the binding to the manifest
//! editor instead.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    Subscription, Task, Window,
};
use oxikube_app::ClusterSessionManager;
use oxikube_app::binding_ops::BindingChoices;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::rbac::{Binding, BindingKind, RoleRef};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::InputState;
use oxikube_workspace::CommandDispatcher;
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::dialog_edit::SubjectEditor;
use super::draft::{BindingDraft, Field, Problem, SubjectDraft};
use crate::role::{Level, Load};

/// What the dialog opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogMode {
    /// A new RoleBinding in `namespace`, or a new ClusterRoleBinding.
    New {
        /// RoleBinding or ClusterRoleBinding.
        kind: BindingKind,
        /// The namespace a new RoleBinding starts in.
        namespace: String,
    },
    /// An existing binding, read fresh.
    Edit(ResourceRef),
}

/// What the dialog is built over.
#[derive(Clone)]
pub struct BindingDialogDeps {
    /// The sessions: the cluster's reader.
    pub sessions: ClusterSessionManager,
    /// Where the save goes (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The cluster.
    pub cluster: ClusterId,
}

/// The binding dialog. See the module docs.
pub struct BindingDialog {
    pub(super) deps: BindingDialogDeps,
    pub(super) mode: DialogMode,
    pub(super) kind: BindingKind,
    pub(super) load: Load,
    pub(super) name: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) role: Option<RoleRef>,
    /// The role picker's filter.
    pub(super) role_query: Entity<InputState>,
    /// The role picker is open (no role yet, or "Change" was pressed).
    pub(super) role_open: bool,
    pub(super) subjects: Vec<SubjectEditor>,
    pub(super) base: Option<serde_json::Value>,
    pub(super) choices: BindingChoices,
    /// The pickers' lists are still being read (the form is usable meanwhile).
    pub(super) choices_pending: bool,
    /// The subject whose name suggestions are shown.
    pub(super) picking: Option<usize>,
    /// The user tried to save: errors show.
    pub(super) attempted: bool,
    pub(super) focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _load: Task<()>,
    _choices: Task<()>,
}

impl EventEmitter<DismissEvent> for BindingDialog {}

impl Focusable for BindingDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for BindingDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Typed subjects are not lost to a stray click on the scrim.
    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        false
    }
}

impl BindingDialog {
    /// A dialog for `mode`, reading what it needs off the UI thread.
    pub fn new(
        deps: BindingDialogDeps,
        mode: DialogMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (kind, namespace) = match &mode {
            DialogMode::New { kind, namespace } => (*kind, namespace.clone()),
            DialogMode::Edit(target) => (
                if &*target.gvk.kind == "ClusterRoleBinding" {
                    BindingKind::ClusterRoleBinding
                } else {
                    BindingKind::RoleBinding
                },
                target.namespace().unwrap_or_default().to_owned(),
            ),
        };
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("my-binding"));
        let namespace_input = cx.new(|cx| InputState::new(window, cx).placeholder("default"));
        namespace_input.update(cx, |input, cx| input.set_value(namespace, window, cx));
        let role_query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a role…"));
        let subscriptions = vec![
            cx.subscribe_in(&namespace_input, window, Self::on_input),
            cx.subscribe_in(&role_query, window, Self::on_input),
        ];
        let load = Self::start_load(&deps, &mode, cx);
        let choices = Self::start_choices(&deps, cx);
        let editing = matches!(mode, DialogMode::Edit(_));
        let mut dialog = Self {
            deps,
            mode,
            kind,
            // A new binding is ready at once; an edit waits for the live object only.
            load: if editing { Load::Loading } else { Load::Ready },
            name,
            namespace: namespace_input,
            role: None,
            role_query,
            role_open: true,
            subjects: Vec::new(),
            base: None,
            choices: BindingChoices::default(),
            choices_pending: true,
            picking: None,
            attempted: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            _load: load,
            _choices: choices,
        };
        dialog.add_subject(&SubjectDraft::default(), window, cx);
        dialog
    }

    /// Reads the live binding (editing) off the UI thread; the form shows it as soon as it
    /// arrives, without waiting for the pickers' lists.
    fn start_load(deps: &BindingDialogDeps, mode: &DialogMode, cx: &mut Context<Self>) -> Task<()> {
        let DialogMode::Edit(target) = mode else {
            return Task::ready(());
        };
        let target = target.clone();
        let reader = deps
            .sessions
            .get(&deps.cluster)
            .and_then(|session| session.resources());
        let read = spawn_kube(cx, async move {
            let reader =
                reader.ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
            reader
                .get(&target.gvk, target.namespace(), &target.name)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::internal(error.to_string())),
            };
            this.update_in(cx, |this, window, cx| this.loaded(result, window, cx))
                .ok();
        })
    }

    /// Reads the pickers' choices off the UI thread (five lists at once).
    fn start_choices(deps: &BindingDialogDeps, cx: &mut Context<Self>) -> Task<()> {
        let sessions = deps.sessions.clone();
        let cluster = deps.cluster.clone();
        let read = spawn_kube(cx, async move {
            BindingChoices::read(&sessions, &cluster).await
        });
        cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::internal(error.to_string())),
            };
            this.update(cx, |this, cx| this.choices_read(result, cx))
                .ok();
        })
    }

    fn loaded(&mut self, result: OxiResult<Resource>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Err(error) => self.load = Load::Failed(error.message().to_owned().into()),
            Ok(live) => {
                self.load = Load::Ready;
                self.fill_from(&live, window, cx);
            }
        }
        cx.notify();
    }

    fn choices_read(&mut self, result: OxiResult<BindingChoices>, cx: &mut Context<Self>) {
        self.choices_pending = false;
        match result {
            Ok(choices) => self.choices = choices,
            // Only a cluster that is not connected fails as a whole: nothing can be saved then.
            Err(error) => self.load = Load::Failed(error.message().to_owned().into()),
        }
        cx.notify();
    }

    /// Replaces the fields with those of the live binding.
    fn fill_from(&mut self, live: &Resource, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(binding) = Binding::from_resource(live) else {
            self.load =
                Load::Failed("The object is not a RoleBinding or ClusterRoleBinding".into());
            return;
        };
        let draft = BindingDraft::edit(&binding, live.to_value());
        self.name.update(cx, |input, cx| {
            input.set_value(draft.name.clone(), window, cx);
        });
        self.role = draft.role.clone();
        self.role_open = false;
        self.subjects.clear();
        for subject in &draft.subjects {
            self.add_subject(subject, window, cx);
        }
        self.base = draft.base;
    }

    /// The binding as typed.
    pub fn draft(&self, cx: &App) -> BindingDraft {
        BindingDraft {
            kind: self.kind,
            namespace: self.namespace.read(cx).value().to_string(),
            name: self.name.read(cx).value().to_string(),
            role: self.role.clone(),
            subjects: self
                .subjects
                .iter()
                .map(|subject| subject.draft(cx))
                .collect(),
            base: self.base.clone(),
        }
    }

    /// What is wrong with the binding as typed; errors only once the user tried to save.
    pub fn problems(&self, cx: &App) -> Vec<Problem> {
        let all = self.draft(cx).problems();
        if self.attempted {
            all
        } else {
            all.into_iter()
                .filter(|problem| problem.level == Level::Warning)
                .collect()
        }
    }

    /// The problem shown for `field`, if there is one.
    pub fn problem_at(&self, field: Field, cx: &App) -> Option<Problem> {
        self.problems(cx)
            .into_iter()
            .find(|problem| problem.at == field)
    }

    /// Whether this edits an existing binding.
    pub fn is_edit(&self) -> bool {
        matches!(self.mode, DialogMode::Edit(_))
    }

    /// Whether saving makes a new binding because the role changed.
    pub fn recreates(&self, cx: &App) -> bool {
        self.draft(cx).role_changed()
    }

    /// Where the dialog's reads are.
    pub fn load_state(&self) -> &Load {
        &self.load
    }

    /// Checks the binding; a valid one is sent as `binding::Create` or `binding::Edit` and the
    /// dialog closes, otherwise each wrong field is marked and nothing is sent.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.attempted = true;
        if self.load != Load::Ready {
            cx.notify();
            return;
        }
        let draft = self.draft(cx);
        if draft.has_errors() {
            cx.notify();
            return;
        }
        let manifest = draft.manifest().to_string();
        let cluster = self.deps.cluster.clone();
        let command = if self.is_edit() {
            Command::BindingEdit {
                cluster,
                manifest,
                previous: draft
                    .role_changed()
                    .then(|| draft.base.as_ref().map(ToString::to_string))
                    .flatten(),
            }
        } else {
            Command::BindingCreate { cluster, manifest }
        };
        self.deps.dispatcher.dispatch(command, cx);
        cx.emit(DismissEvent);
    }

    /// "Edit as YAML": the manifest editor on this binding (or a template for a new one).
    pub fn edit_as_yaml(&mut self, cx: &mut Context<Self>) {
        let command = match &self.mode {
            DialogMode::Edit(target) => Command::ResourceEdit {
                target: target.clone(),
            },
            DialogMode::New { kind, .. } => Command::ResourceCreate {
                cluster: Some(self.deps.cluster.clone()),
                template: Some(kind.as_str().to_owned()),
            },
        };
        self.deps.dispatcher.dispatch(command, cx);
        cx.emit(DismissEvent);
    }

    /// Closes the dialog without sending anything.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}
