//! [`RoleDialog`]: the Role / ClusterRole rule editor.
//!
//! Rows of rules, each with API groups, a resource picker backed by the cluster's discovery, verb
//! checkboxes and optional resource names (and URLs in a ClusterRole). It builds the object as
//! JSON ([`RoleDraft::manifest`]) and sends it as `role::Create` or `role::Edit`: the guard
//! confirms it (risk medium), the handler dry-runs it and writes it. The dialog never calls a
//! port that writes. "Edit as YAML" hands the role to the manifest editor instead.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Task, Window,
};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::rbac::{RbacKind, Role, RoleKind};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::InputState;
use oxikube_workspace::CommandDispatcher;
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::draft::{Field, Level, Problem, RoleDraft, RuleDraft};
use super::options::ResourceOptions;
use super::rule_editor::RuleEditor;

/// What the dialog opens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogMode {
    /// A new Role in `namespace`, or a new ClusterRole.
    New {
        /// Role or ClusterRole.
        kind: RoleKind,
        /// The namespace a new Role starts in.
        namespace: String,
    },
    /// The rules of an existing Role or ClusterRole, read fresh.
    Edit(ResourceRef),
}

/// What the dialog is built over.
#[derive(Clone)]
pub struct RoleDialogDeps {
    /// The sessions: the cluster's reader and discovery.
    pub sessions: ClusterSessionManager,
    /// Where the save goes (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The cluster.
    pub cluster: ClusterId,
}

/// Where the dialog's reads are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    /// Reading the role and the cluster's resources.
    Loading,
    /// Ready to edit.
    Ready,
    /// The role could not be read.
    Failed(SharedString),
}

/// What the off-thread reads bring back.
struct Loaded {
    live: Option<Resource>,
    kinds: OxiResult<Vec<ResourceKind>>,
}

/// The rule dialog. See the module docs.
pub struct RoleDialog {
    pub(super) deps: RoleDialogDeps,
    pub(super) mode: DialogMode,
    pub(super) kind: RoleKind,
    pub(super) load: Load,
    pub(super) name: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) rules: Vec<RuleEditor>,
    pub(super) base: Option<serde_json::Value>,
    pub(super) options: ResourceOptions,
    /// Discovery failed or is not connected: names are typed, not picked.
    pub(super) discovery_note: Option<SharedString>,
    /// The rule whose resource picker is open.
    pub(super) picking: Option<usize>,
    /// The user tried to save: errors show.
    pub(super) attempted: bool,
    pub(super) focus: FocusHandle,
    _load: Task<()>,
}

impl EventEmitter<DismissEvent> for RoleDialog {}

impl Focusable for RoleDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for RoleDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Typed rules are not lost to a stray click on the scrim.
    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        false
    }
}

impl RoleDialog {
    /// A dialog for `mode`, reading what it needs off the UI thread.
    pub fn new(
        deps: RoleDialogDeps,
        mode: DialogMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (kind, namespace) = match &mode {
            DialogMode::New { kind, namespace } => (*kind, namespace.clone()),
            DialogMode::Edit(target) => (
                if &*target.gvk.kind == "ClusterRole" {
                    RoleKind::ClusterRole
                } else {
                    RoleKind::Role
                },
                target.namespace().unwrap_or_default().to_owned(),
            ),
        };
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("my-role"));
        let namespace_input = cx.new(|cx| InputState::new(window, cx).placeholder("default"));
        namespace_input.update(cx, |input, cx| input.set_value(namespace, window, cx));
        let load = Self::start_load(&deps, &mode, cx);
        let mut dialog = Self {
            deps,
            mode,
            kind,
            load: Load::Loading,
            name,
            namespace: namespace_input,
            rules: Vec::new(),
            base: None,
            options: ResourceOptions::default(),
            discovery_note: None,
            picking: None,
            attempted: false,
            focus: cx.focus_handle(),
            _load: load,
        };
        dialog.add_rule(&RuleDraft::empty(), window, cx);
        dialog
    }

    /// Reads the live role (editing) and the cluster's kinds, off the UI thread.
    fn start_load(deps: &RoleDialogDeps, mode: &DialogMode, cx: &mut Context<Self>) -> Task<()> {
        let session = deps.sessions.get(&deps.cluster);
        let reader = session.as_ref().and_then(|session| session.resources());
        let discovery = session.as_ref().and_then(|session| session.discovery());
        let target = match mode {
            DialogMode::Edit(target) => Some(target.clone()),
            DialogMode::New { .. } => None,
        };
        let read = spawn_kube(cx, async move {
            let live = match (&target, reader) {
                (Some(target), Some(reader)) => Some(
                    reader
                        .get(&target.gvk, target.namespace(), &target.name)
                        .await?,
                ),
                (Some(_), None) => return Err(OxiError::conflict("the cluster is not connected")),
                (None, _) => None,
            };
            let kinds = match discovery {
                Some(discovery) => discovery.discover().await,
                None => Err(OxiError::conflict("the cluster is not connected")),
            };
            Ok(Loaded { live, kinds })
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

    fn loaded(&mut self, result: OxiResult<Loaded>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Err(error) => self.load = Load::Failed(error.message().to_owned().into()),
            Ok(loaded) => {
                match loaded.kinds {
                    Ok(kinds) => self.options = ResourceOptions::from_kinds(&kinds),
                    Err(error) => {
                        self.discovery_note = Some(
                            format!(
                                "The cluster's resources could not be listed ({}): type names instead",
                                error.message()
                            )
                            .into(),
                        );
                    }
                }
                if let Some(live) = loaded.live {
                    self.fill_from(&live, window, cx);
                }
                self.load = Load::Ready;
            }
        }
        cx.notify();
    }

    /// Replaces the rows with those of the live role.
    fn fill_from(&mut self, live: &Resource, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(role) = Role::from_resource(live) else {
            self.load = Load::Failed("The object is not a Role or ClusterRole".into());
            return;
        };
        let draft = RoleDraft::edit(&role, live.to_value());
        self.name.update(cx, |input, cx| {
            input.set_value(draft.name.clone(), window, cx);
        });
        self.rules.clear();
        for rule in &draft.rules {
            self.add_rule(rule, window, cx);
        }
        self.base = draft.base;
    }

    /// Adds a row filled with `rule`.
    pub fn add_rule(&mut self, rule: &RuleDraft, window: &mut Window, cx: &mut Context<Self>) {
        let editor = RuleEditor::new(rule, window, cx);
        self.rules.push(editor);
        cx.notify();
    }

    /// Replaces row `ix` (adding it when there are fewer rows) with `rule`, as typed.
    pub fn fill_rule(
        &mut self,
        ix: usize,
        rule: &RuleDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix < self.rules.len() {
            self.rules[ix] = RuleEditor::new(rule, window, cx);
            cx.notify();
        } else {
            self.add_rule(rule, window, cx);
        }
    }

    /// Fills the name, as typed.
    pub fn fill_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
    }

    /// Removes row `ix`.
    pub fn remove_rule(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.rules.len() {
            self.rules.remove(ix);
            self.picking = None;
            cx.notify();
        }
    }

    /// Checks or unchecks `verb` of row `ix`.
    pub fn set_verb(&mut self, ix: usize, verb: &str, on: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.rules.get_mut(ix) else {
            return;
        };
        let mut rule = RuleDraft {
            verbs: std::mem::take(&mut editor.verbs),
            ..RuleDraft::empty()
        };
        rule.set_verb(verb, on);
        editor.verbs = rule.verbs;
        self.picking = None;
        cx.notify();
    }

    /// Picks `resource` of `group` into row `ix`.
    pub fn pick(
        &mut self,
        ix: usize,
        group: &str,
        resource: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.rules.get(ix) {
            editor.pick(group, resource, window, cx);
        }
        self.picking = None;
        cx.notify();
    }

    /// The role as typed.
    pub fn draft(&self, cx: &App) -> RoleDraft {
        RoleDraft {
            kind: self.kind,
            namespace: self.namespace.read(cx).value().to_string(),
            name: self.name.read(cx).value().to_string(),
            rules: self.rules.iter().map(|rule| rule.draft(cx)).collect(),
            base: self.base.clone(),
        }
    }

    /// What is wrong with the role as typed; errors only once the user tried to save.
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

    /// Whether the control plane owns the rules, so the rows are read-only.
    pub fn rules_locked(&self) -> bool {
        self.base.as_ref().is_some_and(|base| {
            base.get("aggregationRule")
                .is_some_and(serde_json::Value::is_object)
        })
    }

    /// Whether this edits an existing role.
    pub fn is_edit(&self) -> bool {
        matches!(self.mode, DialogMode::Edit(_))
    }

    /// Where the dialog's reads are.
    pub fn load_state(&self) -> &Load {
        &self.load
    }

    /// Checks the role; a valid one is sent as `role::Create` or `role::Edit` and the dialog
    /// closes, otherwise each wrong field is marked and nothing is sent.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.attempted = true;
        if self.load != Load::Ready || self.rules_locked() {
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
            Command::RoleEdit { cluster, manifest }
        } else {
            Command::RoleCreate { cluster, manifest }
        };
        self.deps.dispatcher.dispatch(command, cx);
        cx.emit(DismissEvent);
    }

    /// "Edit as YAML": the manifest editor on this role (or a template for a new one).
    pub fn edit_as_yaml(&mut self, cx: &mut Context<Self>) {
        let command = match &self.mode {
            DialogMode::Edit(target) => Command::ResourceEdit {
                target: target.clone(),
            },
            DialogMode::New { kind, .. } => Command::ResourceCreate {
                cluster: Some(self.deps.cluster.clone()),
                template: Some(RbacKind::gvk(kind.rbac_kind()).kind.to_string()),
            },
        };
        self.deps.dispatcher.dispatch(command, cx);
        cx.emit(DismissEvent);
    }

    /// Closes the dialog without sending anything.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// The problem shown for `field`, if there is one.
    pub fn problem_at(&self, field: Field, cx: &App) -> Option<Problem> {
        self.problems(cx)
            .into_iter()
            .find(|problem| problem.at == field)
    }
}
