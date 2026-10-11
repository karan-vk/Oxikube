//! [`CreateSecretDialog`]: the fields of a new Secret and their checks.
//!
//! Three kinds, as `kubectl create secret` has them: Opaque (key and value rows), Docker registry
//! (server, username, password, email) and TLS (a certificate and a private key as PEM). Submitting
//! checks everything through `oxikube_app::SecretService` and marks each field that is wrong; a
//! valid Secret is handed to the sink as an object (`data` base64), which opens it in a manifest
//! editor to review and apply (`resource::Apply`). The dialog never sends anything itself.
//!
//! The values live in the inputs and nowhere else. They are cleared when the dialog closes, and
//! nothing here is logged (`NewSecret`'s `Debug` prints `***`).

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    Window,
};
use oxikube_app::{NewSecret, NewSecretField, NewSecretProblem, SecretService};
use oxikube_domain::secret::SecretText;
use oxikube_ui::input::{InputEvent, InputState, TextareaState};
use oxikube_workspace::modal::{ModalPlacement, ModalView};
use serde_json::Value;

/// What the dialog does with a valid Secret: opens it in an editor.
pub type SecretSink = Rc<dyn Fn(Value, &mut Window, &mut App)>;

/// The kind of Secret being made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    /// `type: Opaque`: key and value rows.
    Opaque,
    /// `type: kubernetes.io/dockerconfigjson`.
    Docker,
    /// `type: kubernetes.io/tls`.
    Tls,
}

impl SecretKind {
    /// Every kind, in the order the dialog lists them.
    pub const ALL: [SecretKind; 3] = [SecretKind::Opaque, SecretKind::Docker, SecretKind::Tls];

    /// The label of the kind's button.
    pub fn label(self) -> &'static str {
        match self {
            SecretKind::Opaque => "Opaque",
            SecretKind::Docker => "Docker registry",
            SecretKind::Tls => "TLS",
        }
    }
}

/// One key and value of an Opaque Secret.
pub(super) struct Row {
    pub(super) key: Entity<InputState>,
    pub(super) value: Entity<TextareaState>,
}

/// The "Create secret" dialog. See the module docs.
pub struct CreateSecretDialog {
    pub(super) kind: SecretKind,
    pub(super) name: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) rows: Vec<Row>,
    pub(super) server: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) email: Entity<InputState>,
    pub(super) certificate: Entity<TextareaState>,
    pub(super) private_key: Entity<TextareaState>,
    /// What was wrong at the last submit, cleared by the next edit.
    pub(super) problems: Vec<NewSecretProblem>,
    service: SecretService,
    choose: SecretSink,
    pub(super) focus: FocusHandle,
    subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<DismissEvent> for CreateSecretDialog {}

impl Focusable for CreateSecretDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for CreateSecretDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Typed values are not lost to a stray click on the scrim.
    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        false
    }

    /// Closing the dialog forgets what was typed.
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.clear_values(window, cx);
        true
    }
}

impl CreateSecretDialog {
    /// A dialog for a new Secret in `namespace`; a valid Secret goes to `choose`.
    pub fn new(
        namespace: &str,
        choose: SecretSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = input("my-secret", window, cx);
        let namespace_input = input("default", window, cx);
        namespace_input.update(cx, |input, cx| {
            input.set_value(namespace.to_owned(), window, cx);
        });
        let server = input("ghcr.io", window, cx);
        let username = input("username", window, cx);
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("password or token")
                .masked(true)
        });
        let email = input("optional", window, cx);
        let certificate = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("-----BEGIN CERTIFICATE-----")
                .rows(6)
        });
        let private_key = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("-----BEGIN PRIVATE KEY-----")
                .rows(6)
        });
        let mut dialog = Self {
            kind: SecretKind::Opaque,
            name,
            namespace: namespace_input,
            rows: Vec::new(),
            server,
            username,
            password,
            email,
            certificate,
            private_key,
            problems: Vec::new(),
            service: SecretService::new(),
            choose,
            focus: cx.focus_handle(),
            subscriptions: Vec::new(),
        };
        dialog.add_row(window, cx);
        dialog.watch_inputs(window, cx);
        dialog.name.update(cx, |name, cx| name.focus(window, cx));
        dialog
    }

    /// Clears the problems when a field is edited, and submits on Enter in a one-line field.
    fn watch_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = [
            self.name.clone(),
            self.namespace.clone(),
            self.server.clone(),
            self.username.clone(),
            self.password.clone(),
            self.email.clone(),
        ];
        for input in &inputs {
            self.subscriptions
                .push(cx.subscribe_in(input, window, Self::on_input));
        }
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if !self.problems.is_empty() {
                    self.problems.clear();
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } if self.kind != SecretKind::Opaque => {
                self.submit(window, cx);
            }
            _ => {}
        }
    }

    /// The kind being made.
    pub fn kind(&self) -> SecretKind {
        self.kind
    }

    /// Switches to `kind`; what was typed for the others stays.
    pub fn select_kind(&mut self, kind: SecretKind, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            self.problems.clear();
            cx.notify();
        }
    }

    /// How many key and value rows an Opaque Secret has.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Adds an empty row.
    pub fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = cx.new(|cx| InputState::new(window, cx).placeholder("key"));
        let value = cx.new(|cx| TextareaState::new(window, cx).placeholder("value").rows(2));
        self.subscriptions
            .push(cx.subscribe_in(&key, window, Self::on_input));
        self.rows.push(Row { key, value });
        cx.notify();
    }

    /// Removes row `index` (the last row stays: an Opaque Secret needs one to start from).
    pub fn remove_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.rows.len() > 1 && index < self.rows.len() {
            self.rows.remove(index);
            self.problems.clear();
            cx.notify();
        }
    }

    /// The problem shown for `field`, if the last submit found one.
    pub fn problem(&self, field: NewSecretField) -> Option<&str> {
        self.problems
            .iter()
            .find(|problem| problem.field == field)
            .map(|problem| problem.message.as_str())
    }

    /// Every problem the last submit found.
    pub fn problems(&self) -> &[NewSecretProblem] {
        &self.problems
    }

    /// The rows that make a key, by index into the rows: a row left empty is not one.
    fn kept_rows(&self, cx: &App) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&index| {
                let row = &self.rows[index];
                !(row.key.read(cx).value().trim().is_empty()
                    && row.value.read(cx).value().is_empty())
            })
            .collect()
    }

    /// `problems`, found in the draft's entries, moved to the rows they came from (the draft
    /// leaves empty rows out, so its indices are not the rows').
    fn problems_on_rows(&self, problems: Vec<NewSecretProblem>, cx: &App) -> Vec<NewSecretProblem> {
        if self.kind != SecretKind::Opaque {
            return problems;
        }
        let kept = self.kept_rows(cx);
        let row = |index: usize| kept.get(index).copied().unwrap_or(index);
        problems
            .into_iter()
            .map(|mut problem| {
                problem.field = match problem.field {
                    NewSecretField::Key(index) => NewSecretField::Key(row(index)),
                    NewSecretField::Value(index) => NewSecretField::Value(row(index)),
                    other => other,
                };
                problem
            })
            .collect()
    }

    /// The Secret the fields describe.
    pub fn draft(&self, cx: &App) -> NewSecret {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let name = text(&self.name);
        let namespace = text(&self.namespace);
        match self.kind {
            SecretKind::Opaque => NewSecret::Opaque {
                name,
                namespace,
                entries: self
                    .kept_rows(cx)
                    .into_iter()
                    .map(|index| {
                        let row = &self.rows[index];
                        (
                            text(&row.key),
                            SecretText::new(row.value.read(cx).value().to_string()),
                        )
                    })
                    .collect(),
            },
            SecretKind::Docker => NewSecret::DockerRegistry {
                name,
                namespace,
                server: text(&self.server),
                username: self.username.read(cx).value().to_string(),
                password: SecretText::new(self.password.read(cx).value().to_string()),
                email: text(&self.email),
            },
            SecretKind::Tls => NewSecret::Tls {
                name,
                namespace,
                certificate: SecretText::new(self.certificate.read(cx).value().to_string()),
                private_key: SecretText::new(self.private_key.read(cx).value().to_string()),
            },
        }
    }

    /// Checks the fields; a valid Secret goes to the sink and the dialog closes, otherwise each
    /// wrong field is marked and nothing is sent.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        match self.service.manifest(&draft) {
            Ok(manifest) => {
                self.clear_values(window, cx);
                self.problems.clear();
                // The editor opens on the next turn, after this (as the template picker's choice
                // does); the dialog closes after it is asked, so the new tab keeps the focus.
                (self.choose)(manifest, window, cx);
                cx.emit(DismissEvent);
            }
            Err(problems) => {
                self.problems = self.problems_on_rows(problems, cx);
                cx.notify();
            }
        }
    }

    /// Closes the dialog without making anything.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_values(window, cx);
        cx.emit(DismissEvent);
    }

    /// Empties every field that can hold a value, so nothing typed outlives the dialog.
    fn clear_values(&mut self, window: &mut Window, cx: &mut App) {
        for input in [&self.password, &self.username, &self.server] {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        for area in [&self.certificate, &self.private_key] {
            area.update(cx, |area, cx| area.set_value(String::new(), window, cx));
        }
        for row in &self.rows {
            row.value
                .update(cx, |area, cx| area.set_value(String::new(), window, cx));
        }
    }

    /// Fills the name and namespace, as typed.
    pub fn fill_identity(
        &mut self,
        name: &str,
        namespace: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
        self.namespace.update(cx, |input, cx| {
            input.set_value(namespace.to_owned(), window, cx);
        });
    }

    /// Fills row `index` (adding rows as needed), as typed.
    pub fn fill_row(
        &mut self,
        index: usize,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while self.rows.len() <= index {
            self.add_row(window, cx);
        }
        let row = &self.rows[index];
        row.key
            .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
        row.value
            .update(cx, |area, cx| area.set_value(value.to_owned(), window, cx));
    }

    /// Fills the registry fields, as typed.
    pub fn fill_registry(
        &mut self,
        server: &str,
        username: &str,
        password: &str,
        email: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (input, text) in [
            (&self.server, server),
            (&self.username, username),
            (&self.password, password),
            (&self.email, email),
        ] {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// Fills the PEM fields, as pasted.
    pub fn fill_tls(&mut self, certificate: &str, key: &str, window: &mut Window, cx: &mut App) {
        self.certificate.update(cx, |area, cx| {
            area.set_value(certificate.to_owned(), window, cx)
        });
        self.private_key
            .update(cx, |area, cx| area.set_value(key.to_owned(), window, cx));
    }
}
