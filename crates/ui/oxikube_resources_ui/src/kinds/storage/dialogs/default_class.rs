//! [`DefaultClassDialog`]: make a StorageClass the cluster's default.
//!
//! A small modal: it reads the classes off the UI thread, says which class is the default now
//! (a cluster should have one), offers to clear it (on by default), and sends
//! `storageclass::SetDefault` through the bus when the button is pressed. The guard's simple
//! confirmation is that button.

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Task, WeakEntity, Window,
    div, px,
};
use oxikube_app::{DefaultClassContext, DefaultClassFlow};
use oxikube_domain::ids::ResourceRef;
use oxikube_runtime::spawn_kube;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};
use oxikube_workspace::{Toast, Workspace};

use super::shared::{settle, show_toast};

/// What the dialog has read.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassLoaded {
    /// Reading the classes.
    Loading,
    /// Read.
    Ready(Box<DefaultClassContext>),
    /// The read failed, with the reason.
    Failed(SharedString),
}

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassStage {
    /// Asking. Nothing has been sent.
    Asking,
    /// Running.
    Running,
    /// It went through; the dialog is closing.
    Done,
}

/// The set-default dialog. See the module docs.
pub struct DefaultClassDialog {
    flow: DefaultClassFlow,
    loaded: ClassLoaded,
    stage: ClassStage,
    clear_previous: bool,
    error: Option<SharedString>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    _load: Task<()>,
}

impl EventEmitter<DismissEvent> for DefaultClassDialog {}

impl Focusable for DefaultClassDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for DefaultClassDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.stage != ClassStage::Running
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        self.stage != ClassStage::Running
    }
}

impl DefaultClassDialog {
    /// A dialog for `target`, reading the classes now.
    pub fn new(
        flow: DefaultClassFlow,
        target: ResourceRef,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let load = {
            let flow = flow.clone();
            cx.spawn(async move |this, cx| {
                let read = spawn_kube(cx, async move { flow.open(&target).await }).await;
                let loaded = match settle(read) {
                    Ok(context) => ClassLoaded::Ready(Box::new(context)),
                    Err(reason) => ClassLoaded::Failed(reason.into()),
                };
                this.update(cx, |this, cx| {
                    this.loaded = loaded;
                    cx.notify();
                })
                .ok();
            })
        };
        Self {
            flow,
            loaded: ClassLoaded::Loading,
            stage: ClassStage::Asking,
            clear_previous: true,
            error: None,
            workspace,
            focus: cx.focus_handle(),
            _load: load,
        }
    }

    /// Focuses the dialog.
    pub fn focus_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// What the dialog has read.
    pub fn loaded(&self) -> &ClassLoaded {
        &self.loaded
    }

    /// Where the dialog is.
    pub fn stage(&self) -> ClassStage {
        self.stage
    }

    /// The last failure, shown under the choices.
    pub fn error_text(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the previous default will be cleared too.
    pub fn clears_previous(&self) -> bool {
        self.clear_previous
    }

    /// Chooses whether the previous default is cleared too.
    pub fn set_clear_previous(&mut self, clear: bool, cx: &mut Context<Self>) {
        self.clear_previous = clear;
        cx.notify();
    }

    /// Whether there is anything to do: read, and not the only default already.
    pub fn can_confirm(&self) -> bool {
        self.stage == ClassStage::Asking
            && matches!(&self.loaded, ClassLoaded::Ready(context) if !context.is_noop())
    }

    /// Cancels while asking.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != ClassStage::Running {
            cx.emit(DismissEvent);
        }
    }

    /// Makes the class the default.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let ClassLoaded::Ready(context) = &self.loaded else {
            return;
        };
        if !self.can_confirm() {
            return;
        }
        let context = (**context).clone();
        let clear = self.clear_previous;
        self.stage = ClassStage::Running;
        self.error = None;
        cx.notify();
        let flow = self.flow.clone();
        let workspace = self.workspace.clone();
        // Detached for the same reason as the delete: it is audited as it runs.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { flow.run(&context, clear).await });
            let result = settle(run.await).map(|report| report.message);
            let delivered = this.update(cx, |this, cx| match &result {
                Ok(message) => {
                    this.stage = ClassStage::Done;
                    show_toast(&this.workspace, Toast::success(message.clone()), cx);
                    cx.emit(DismissEvent);
                }
                Err(message) => {
                    this.stage = ClassStage::Asking;
                    this.error = Some(message.clone().into());
                    cx.notify();
                }
            });
            if delivered.is_err() {
                let toast = match &result {
                    Ok(message) => Toast::success(message.clone()),
                    Err(message) => Toast::error(message.clone()),
                };
                show_toast(&workspace, toast, cx);
            }
        })
        .detach();
    }

    fn body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = cx.colors();
        match &self.loaded {
            ClassLoaded::Loading => h_flex()
                .debug_selector(|| "default-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Reading the storage classes…")
                .into_any_element(),
            ClassLoaded::Failed(reason) => div()
                .debug_selector(|| "default-load-error".into())
                .text_color(colors.error)
                .child(reason.clone())
                .into_any_element(),
            ClassLoaded::Ready(context) => {
                let now = match context.previous.as_slice() {
                    [] if context.class.is_default => {
                        format!(
                            "{} is the default storage class already.",
                            context.class.name
                        )
                    }
                    [] => "No storage class is the default now.".to_owned(),
                    previous => format!(
                        "{} {} the default now.",
                        previous
                            .iter()
                            .map(|n| n.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        if previous.len() == 1 { "is" } else { "are" }
                    ),
                };
                let choices =
                    (!context.previous.is_empty()).then(|| {
                        let clear = self.clear_previous;
                        h_flex()
                            .debug_selector(|| "default-choice".into())
                            .gap(u(px(4.)))
                            .items_center()
                            .child(
                                div()
                                    .text_color(colors.text_muted)
                                    .child("Previous default"),
                            )
                            .child(
                                Button::new("default-clear")
                                    .label("Clear it")
                                    .small()
                                    .selected(clear)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_clear_previous(true, cx)
                                    })),
                            )
                            .child(
                                Button::new("default-keep")
                                    .label("Keep it")
                                    .small()
                                    .selected(!clear)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_clear_previous(false, cx)
                                    })),
                            )
                    });
                v_flex()
                    .gap(u(px(10.)))
                    .child(
                        div()
                            .debug_selector(|| "default-what".into())
                            .text_color(colors.text_muted)
                            .child(format!("{} on {}", context.class.name, context.context)),
                    )
                    .child(div().debug_selector(|| "default-now".into()).child(now))
                    .children(choices)
                    .children(self.error.clone().map(|error| {
                        div()
                            .debug_selector(|| "default-error".into())
                            .text_color(colors.error)
                            .child(error)
                    }))
                    .into_any_element()
            }
        }
    }
}

impl Render for DefaultClassDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let title = match &self.loaded {
            ClassLoaded::Ready(context) => format!("Make {} the default?", context.class.name),
            _ => "Set the default storage class".to_owned(),
        };
        let enabled = self.can_confirm();
        let footer = DialogFooter::new()
            .child(
                Button::new("default-cancel")
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "default-confirm".into()).child(
                    Button::new("default-confirm")
                        .label("Set as Default")
                        .primary()
                        .disabled(!enabled)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                ),
            );
        v_flex()
            .id("default-class-dialog")
            .debug_selector(|| "default-class-dialog".into())
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
            .on_action(cx.listener(|this, _: &oxikube_ui::dialog::Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &oxikube_ui::dialog::Confirm, _, cx| this.confirm(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(title)))
            .child(self.body(cx))
            .child(footer)
    }
}
