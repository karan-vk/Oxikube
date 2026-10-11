//! [`CreateNamespaceDialog`]: a name and optional labels, checked as they are typed, created
//! through `namespace::Create`.
//!
//! The dialog is the confirmation the guard asks for a low-risk create: it names the cluster and
//! the namespace, and its button is the user's one click. Create runs on the Tokio bridge; a
//! refusal (the name exists, RBAC, an admission policy, a read-only cluster) is shown under the
//! fields and the dialog stays open, so the name can be fixed without typing the rest again.

use std::collections::BTreeMap;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};
use oxikube_app::namespaces::{NamespaceCreator, parse_labels, validate_name};
use oxikube_domain::ids::ClusterId;
use oxikube_runtime::spawn_kube;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateStage {
    /// Asking. Nothing has been sent.
    Editing,
    /// `namespace::Create` is running.
    Creating,
}

/// The create-namespace dialog. See the [module docs](self).
pub struct CreateNamespaceDialog {
    cluster: ClusterId,
    context: SharedString,
    creator: NamespaceCreator,
    name: Entity<InputState>,
    labels: Entity<InputState>,
    stage: CreateStage,
    /// The API server's (or the guard's) refusal of the last submit.
    error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CreateNamespaceDialog {}

impl Focusable for CreateNamespaceDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for CreateNamespaceDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Escape and a click outside cancel while asking; while the create runs the dialog stays
    /// (the request is out; its answer is what the user wants to see).
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage == CreateStage::Editing
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage == CreateStage::Editing
    }
}

impl CreateNamespaceDialog {
    /// A dialog creating in `cluster` (shown as `context`) through `creator`.
    pub fn new(
        cluster: ClusterId,
        context: impl Into<SharedString>,
        creator: NamespaceCreator,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("my-namespace"));
        let labels =
            cx.new(|cx| InputState::new(window, cx).placeholder("team=a, env=dev (optional)"));
        let subscriptions = [&name, &labels]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into();
        Self {
            cluster,
            context: context.into(),
            creator,
            name,
            labels,
            stage: CreateStage::Editing,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the name field.
    pub fn focus_name(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Where the dialog is.
    pub fn stage(&self) -> CreateStage {
        self.stage
    }

    /// The refusal under the fields, if the last submit failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Types `name` and `labels` into the fields, as the user would.
    pub fn fill(&mut self, name: &str, labels: &str, window: &mut Window, cx: &mut Context<Self>) {
        for (input, text) in [(&self.name, name), (&self.labels, labels)] {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_owned()
    }

    /// Why the name typed so far cannot be used: `None` when it is empty (nothing to complain
    /// about yet) or valid.
    pub fn name_problem(&self, cx: &App) -> Option<String> {
        let name = Self::text(&self.name, cx);
        if name.is_empty() {
            return None;
        }
        validate_name(&name).err().map(|e| e.message().to_owned())
    }

    /// Why the labels typed so far cannot be used.
    pub fn labels_problem(&self, cx: &App) -> Option<String> {
        parse_labels(&Self::text(&self.labels, cx))
            .err()
            .map(|e| e.to_string())
    }

    /// Whether Create can be pressed: asking, a valid name and valid labels.
    pub fn can_submit(&self, cx: &App) -> bool {
        self.stage == CreateStage::Editing
            && !Self::text(&self.name, cx).is_empty()
            && self.name_problem(cx).is_none()
            && self.labels_problem(cx).is_none()
    }

    /// Cancels while asking: nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage == CreateStage::Editing {
            cx.emit(DismissEvent);
        }
    }

    /// Creates the namespace: runs `namespace::Create` on the Tokio bridge. Does nothing while the
    /// name or the labels are invalid (the fields say why).
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            cx.notify();
            return;
        }
        let name = Self::text(&self.name, cx);
        let Ok(labels) = parse_labels(&Self::text(&self.labels, cx)) else {
            return;
        };
        self.stage = CreateStage::Creating;
        self.error = None;
        cx.notify();
        let (creator, cluster) = (self.creator.clone(), self.cluster.clone());
        let workspace = self.workspace.clone();
        // Detached: the command must finish even if the dialog goes away meanwhile (it is
        // audited as it runs); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move {
                let labels: BTreeMap<String, String> = labels;
                creator.create(&cluster, &name, labels).await
            });
            let result = match run.await {
                Ok(result) => result.map_err(|error| error.message().to_owned()),
                Err(error) => Err(error.to_string()),
            };
            let delivered = this.update(cx, |this, cx| this.finished(&result, cx));
            if delivered.is_err() {
                let toast = match &result {
                    Ok(message) => Toast::success(message.clone()),
                    Err(message) => Toast::error(message.clone()),
                };
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finished(&mut self, result: &Result<String, String>, cx: &mut Context<Self>) {
        self.stage = CreateStage::Editing;
        match result {
            Ok(message) => {
                self.workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_toast(Toast::success(message.clone()), cx);
                    })
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(message) => {
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
                self.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.submit(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

impl Render for CreateNamespaceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let creating = self.stage == CreateStage::Creating;
        let labelled = |label: &'static str, id: &'static str, input: &Entity<InputState>| {
            v_flex()
                .gap(u(px(4.)))
                .child(
                    div()
                        .text_size(u(px(12.)))
                        .text_color(colors.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .debug_selector(move || id.into())
                        .child(Input::new(input)),
                )
        };
        let problem = |id: &'static str, text: Option<String>| {
            text.map(|text| {
                div()
                    .debug_selector(move || id.into())
                    .text_size(u(px(12.)))
                    .text_color(colors.error)
                    .child(text)
            })
        };
        let body = v_flex()
            .gap(u(px(12.)))
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child(format!("Creates a namespace on {}.", self.context)),
            )
            .child(labelled("Name", "namespace-create-name", &self.name))
            .children(problem(
                "namespace-create-name-problem",
                self.name_problem(cx),
            ))
            .child(labelled("Labels", "namespace-create-labels", &self.labels))
            .children(problem(
                "namespace-create-labels-problem",
                self.labels_problem(cx),
            ))
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "namespace-create-error".into())
                    .text_color(colors.error)
                    .child(error)
            }))
            .children(creating.then(|| {
                h_flex()
                    .debug_selector(|| "namespace-create-running".into())
                    .gap(u(px(8.)))
                    .items_center()
                    .child(
                        Spinner::new()
                            .icon(Icon::new(IconName::LoaderCircle))
                            .color(colors.accent),
                    )
                    .child("Creating…")
            }));
        let footer = DialogFooter::new()
            .child(
                Button::new("namespace-create-cancel")
                    .label("Cancel")
                    .disabled(creating)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div()
                    .debug_selector(|| "namespace-create-confirm".into())
                    .child(
                        Button::new("namespace-create-confirm")
                            .label("Create")
                            .primary()
                            .disabled(!self.can_submit(cx))
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            );
        v_flex()
            .id("namespace-create-dialog")
            .debug_selector(|| "namespace-create-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(480.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child("Create namespace")))
            .child(body)
            .child(footer)
    }
}
