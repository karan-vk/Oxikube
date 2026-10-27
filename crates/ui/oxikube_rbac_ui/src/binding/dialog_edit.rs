//! Editing the binding dialog's fields: the subject rows, the role pick and the input events.

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use oxikube_domain::rbac::{BindingKind, RoleKind, RoleRef};
use oxikube_ui::input::{InputEvent, InputState};

use super::dialog::BindingDialog;
use super::draft::{SubjectDraft, SubjectKind};

/// One subject's inputs.
pub struct SubjectEditor {
    pub(super) kind: SubjectKind,
    pub(super) name: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl SubjectEditor {
    fn new(subject: &SubjectDraft, window: &mut Window, cx: &mut Context<BindingDialog>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("name"));
        name.update(cx, |input, cx| {
            input.set_value(subject.name.clone(), window, cx);
        });
        let namespace = cx.new(|cx| InputState::new(window, cx).placeholder("namespace"));
        namespace.update(cx, |input, cx| {
            input.set_value(subject.namespace.clone(), window, cx);
        });
        let subscriptions = vec![
            cx.subscribe_in(&name, window, BindingDialog::on_input),
            cx.subscribe_in(&namespace, window, BindingDialog::on_input),
        ];
        Self {
            kind: subject.kind,
            name,
            namespace,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn draft(&self, cx: &App) -> SubjectDraft {
        SubjectDraft {
            kind: self.kind,
            name: self.name.read(cx).value().to_string(),
            namespace: self.namespace.read(cx).value().to_string(),
        }
    }
}

impl BindingDialog {
    /// Any input changed or got the focus: the suggestions follow the subject being typed in, and
    /// a namespace change drops a Role pick that namespace does not have.
    pub(super) fn on_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change | InputEvent::Focus) {
            return;
        }
        let editing = self
            .subjects
            .iter()
            .position(|subject| subject.name == *input || subject.namespace == *input);
        if editing.is_some() || *input != self.role_query {
            self.picking = editing;
        }
        if *input == self.namespace {
            self.drop_role_outside_namespace(cx);
        }
        cx.notify();
    }

    fn drop_role_outside_namespace(&mut self, cx: &App) {
        let namespace = self.namespace.read(cx).value().to_string();
        let gone = self.role.as_ref().is_some_and(|role| {
            role.kind == RoleKind::Role
                && !self
                    .choices
                    .offered_roles(self.role_namespace(&namespace))
                    .iter()
                    .any(|(kind, name)| *kind == RoleKind::Role && *name == role.name)
        });
        if gone {
            self.role = None;
            self.role_open = true;
        }
    }

    /// The namespace a role may be picked from: the RoleBinding's own, none for a
    /// ClusterRoleBinding.
    pub(super) fn role_namespace<'a>(&self, typed: &'a str) -> Option<&'a str> {
        (self.kind == BindingKind::RoleBinding).then_some(typed.trim())
    }

    /// Adds a row filled with `subject`.
    pub fn add_subject(
        &mut self,
        subject: &SubjectDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = SubjectEditor::new(subject, window, cx);
        self.subjects.push(editor);
        cx.notify();
    }

    /// Replaces row `ix` (adding it when there are fewer rows) with `subject`, as typed.
    pub fn fill_subject(
        &mut self,
        ix: usize,
        subject: &SubjectDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix < self.subjects.len() {
            self.subjects[ix] = SubjectEditor::new(subject, window, cx);
            cx.notify();
        } else {
            self.add_subject(subject, window, cx);
        }
    }

    /// Fills the name, as typed.
    pub fn fill_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
    }

    /// Removes row `ix`.
    pub fn remove_subject(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.subjects.len() {
            self.subjects.remove(ix);
            self.picking = None;
            cx.notify();
        }
    }

    /// Makes row `ix` a User, Group or ServiceAccount.
    pub fn set_subject_kind(&mut self, ix: usize, kind: SubjectKind, cx: &mut Context<Self>) {
        if let Some(subject) = self.subjects.get_mut(ix) {
            subject.kind = kind;
            self.picking = Some(ix);
            cx.notify();
        }
    }

    /// Picks the role `kind`/`name`.
    pub fn pick_role(&mut self, kind: RoleKind, name: &str, cx: &mut Context<Self>) {
        self.role = Some(RoleRef {
            kind,
            name: name.into(),
        });
        self.role_open = false;
        cx.notify();
    }

    /// "Change": opens the role picker again.
    pub fn change_role(&mut self, cx: &mut Context<Self>) {
        self.role_open = true;
        cx.notify();
    }

    /// Fills row `ix` with the existing user, group or ServiceAccount `name` of `namespace`.
    pub fn pick_subject(
        &mut self,
        ix: usize,
        name: &str,
        namespace: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(subject) = self.subjects.get(ix) {
            subject
                .name
                .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
            if let Some(namespace) = namespace {
                subject.namespace.update(cx, |input, cx| {
                    input.set_value(namespace.to_owned(), window, cx);
                });
            }
        }
        self.picking = None;
        cx.notify();
    }
}
