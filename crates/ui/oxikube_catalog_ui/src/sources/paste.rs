//! [`PasteDialog`]: a name field, a text area and a warning, hosted by the workspace's modal
//! layer.
//!
//! Submitting sends `kubeconfig::AddSource` with the pasted text. The service validates it
//! first (parsed, no network) and an error comes back as a line inside the dialog, which stays
//! open with the text still in it so the user can fix it. The text goes nowhere else: it is
//! not logged (the command's `Debug` hides it) and the dialog drops it when it closes.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    WeakEntity, Window, div, px,
};
use oxikube_domain::command::{Command, NewKubeconfigSource, PastedText};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::{Input, InputState, Textarea, TextareaState};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::backend::SourcesBackend;
use super::view::SourcesView;

/// The warning under the text area: what storing a pasted kubeconfig means.
pub fn storage_warning(dir: &std::path::Path) -> String {
    format!(
        "Oxikube saves this kubeconfig as a file in {}, readable by you only. A kubeconfig \
         can hold credentials, like the files in ~/.kube. Remove the source to delete the file.",
        dir.display()
    )
}

/// The paste dialog. See the [module docs](self).
pub struct PasteDialog {
    backend: Rc<dyn SourcesBackend>,
    sources: WeakEntity<SourcesView>,
    name: Entity<InputState>,
    text: Entity<TextareaState>,
    error: Option<SharedString>,
    /// A submit is running. A flag, not a task slot: the task that stores the kubeconfig must
    /// finish even if the dialog is dismissed meanwhile, and does not clear itself.
    busy: bool,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for PasteDialog {}

impl Focusable for PasteDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for PasteDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    /// Pasted text is not lost to a stray click on the scrim.
    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        false
    }
}

impl PasteDialog {
    /// A dialog that adds through `backend` and tells `sources` what happened.
    pub fn new(
        backend: Rc<dyn SourcesBackend>,
        sources: WeakEntity<SourcesView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name, for example prod-eu"));
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Paste the kubeconfig here")
                .rows(10)
        });
        let dialog = Self {
            backend,
            sources,
            name,
            text,
            error: None,
            busy: false,
            focus: cx.focus_handle(),
        };
        dialog.name.update(cx, |name, cx| name.focus(window, cx));
        dialog
    }

    /// The name typed so far.
    pub fn name(&self, cx: &App) -> String {
        self.name.read(cx).value().to_string()
    }

    /// The error shown, if any.
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Whether a submit is running.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Sets the name and the text, as if typed.
    pub fn fill(&mut self, name: &str, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
        self.text
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Sends the paste (`kubeconfig::AddSource`). Nothing is sent for an empty name or text: the
    /// dialog says what is missing.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let name = self.name.read(cx).value().trim().to_owned();
        let text = self.text.read(cx).value().to_string();
        if name.is_empty() {
            self.fail("Give the kubeconfig a name.", cx);
            return;
        }
        if text.trim().is_empty() {
            self.fail("Paste the kubeconfig text.", cx);
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = self.backend.run(
            Command::KubeconfigAddSource {
                source: NewKubeconfigSource::Pasted {
                    name,
                    text: PastedText::new(text),
                },
            },
            cx,
        );
        let sources = self.sources.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let dialog_alive = this
                .update(cx, |this, cx| {
                    this.busy = false;
                    match &result {
                        Ok(_) => cx.emit(DismissEvent),
                        Err(error) => this.fail(error.message().to_owned(), cx),
                    }
                })
                .is_ok();
            // The dialog shows its own error. Everything else (a success, or a failure after the
            // dialog was dismissed meanwhile) is told to the screen, so no outcome is lost.
            if result.is_ok() || !dialog_alive {
                sources
                    .update(cx, |view, cx| view.show_outcome(&result, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn fail(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }
}

impl Render for PasteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let warning = storage_warning(&self.backend.stored_dir());
        let busy = self.busy;
        v_flex()
            .id("paste-dialog")
            .debug_selector(|| "paste-dialog".into())
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(560.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .child(DialogHeader::new().child(DialogTitle::new().child("Paste a kubeconfig")))
            .child(
                div()
                    .debug_selector(|| "paste-name".into())
                    .child(Input::new(&self.name)),
            )
            .child(
                div()
                    .debug_selector(|| "paste-text".into())
                    .child(Textarea::new(&self.text).h(u(px(220.)))),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_start()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .debug_selector(|| "paste-warning".into())
                    .child(
                        div().flex_none().child(
                            Icon::new(IconName::KeyRound)
                                .size(u(px(14.)))
                                .color(colors.warning),
                        ),
                    )
                    .child(div().flex_1().child(warning)),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "paste-error".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.error)
                    .child(error)
            }))
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("paste-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        div().debug_selector(|| "paste-submit".into()).child(
                            Button::new("paste-submit")
                                .label("Add")
                                .primary()
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                        ),
                    ),
            )
    }
}
