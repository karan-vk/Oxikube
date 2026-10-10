//! [`ManifestEditor`]: the manifest editor view, a workspace item over [`CodeEditor`].
//!
//! Validation follows the buffer: every change restarts a [`VALIDATION_DEBOUNCE`] timer (the
//! previous task is dropped, which cancels it), then the text is taken as a snapshot (the rope is
//! shared, so this is cheap on the UI thread), parsed and validated on the background executor,
//! and the result is shown only if the buffer is still at the version it was computed for. The
//! schemas a document's kind needs are fetched once per editor through its cluster's
//! `SchemaPort`, on the Kubernetes runtime (`oxikube_runtime::spawn_kube`), and the buffer is
//! validated again when one arrives. Nothing blocks the UI thread, and a keystroke costs the
//! editor's own edit plus a timer restart.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window,
};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ui::ActiveTokens as _;
use oxikube_ui::editor::{CodeEditor, CodeEditorEvent, CodeEditorOptions, EditorApi as _};
use oxikube_ui::layout::v_flex;
use oxikube_workspace::{CommandDispatcher, ItemEvent};

use super::model::ManifestModel;
use super::schemas::SchemaSource;
use super::validation::{VALIDATION_DEBOUNCE, validate_text};
use super::{ToggleReadOnly, ToggleSoftWrap};

/// What a [`ManifestEditor`] opens with.
pub struct ManifestEditorParts {
    /// The tab title ("Untitled-1").
    pub title: SharedString,
    /// The text to start with.
    pub text: String,
    /// The cluster whose schemas the buffer is checked against; `None` checks syntax only.
    pub schemas: Option<Rc<dyn SchemaSource>>,
    /// Where the editor's actions go as commands (`editor::ToggleSoftWrap`, ...): the bus.
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// The manifest editor: a YAML buffer with schema diagnostics, as a workspace item. See the
/// [module docs](self).
pub struct ManifestEditor {
    pub(super) editor: Entity<CodeEditor>,
    pub(super) model: ManifestModel,
    pub(super) title: SharedString,
    pub(super) schemas: Option<Rc<dyn SchemaSource>>,
    dispatcher: Rc<dyn CommandDispatcher>,
    /// The buffer version that counts as unchanged (the text it opened with).
    pub(super) clean_version: u64,
    editor_focused: bool,
    /// The pending or running validation; replacing it cancels the old one.
    validation: Option<Task<()>>,
    /// Schema fetches in flight; dropped (and aborted) with the editor.
    schema_fetches: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ManifestEditor {
    /// A new editor holding `parts.text`, validated at once.
    pub fn new(parts: ManifestEditorParts, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| CodeEditor::new(CodeEditorOptions::YAML, window, cx));
        if !parts.text.is_empty() {
            editor.update(cx, |editor, cx| {
                editor.api(window, cx).set_text(&parts.text)
            });
        }
        let clean_version = editor.read(cx).version();
        let subscriptions = vec![cx.subscribe_in(&editor, window, Self::on_editor_event)];
        let mut this = Self {
            editor,
            model: ManifestModel::new(),
            title: parts.title,
            schemas: parts.schemas,
            dispatcher: parts.dispatcher,
            clean_version,
            editor_focused: false,
            validation: None,
            schema_fetches: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.schedule_validation(Duration::ZERO, window, cx);
        this
    }

    /// The code editor inside.
    pub fn editor(&self) -> &Entity<CodeEditor> {
        &self.editor
    }

    /// The view's state beyond the buffer (problems, schemas).
    pub fn model(&self) -> &ManifestModel {
        &self.model
    }

    /// Whether the buffer differs from what the editor opened with.
    pub fn is_dirty(&self, cx: &gpui::App) -> bool {
        self.editor.read(cx).version() != self.clean_version
    }

    /// Makes the buffer read-only, or editable again (`editor::ToggleReadOnly`).
    pub fn toggle_read_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let mut api = editor.api(window, cx);
            let read_only = !api.is_read_only();
            api.set_read_only(read_only);
        });
        cx.notify();
    }

    /// Wraps long lines, or lets them scroll sideways (`editor::ToggleSoftWrap`).
    pub fn toggle_soft_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let mut api = editor.api(window, cx);
            let wrap = !api.soft_wrap();
            api.set_soft_wrap(wrap);
        });
        cx.notify();
    }

    /// Sends `command` to the bus (the keys, the toolbar and the palette all end up here).
    pub(super) fn send(&self, command: Command, cx: &mut Context<Self>) {
        self.dispatcher.dispatch(command, cx);
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<CodeEditor>,
        event: &CodeEditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CodeEditorEvent::Changed { version } => {
                self.model.changed();
                self.schedule_validation(VALIDATION_DEBOUNCE, window, cx);
                // The dirty dot appears with the first edit.
                if *version == self.clean_version + 1 {
                    cx.emit(ItemEvent::UpdateTab);
                }
                cx.notify();
            }
            CodeEditorEvent::Focused => {
                self.editor_focused = true;
                cx.notify();
            }
            CodeEditorEvent::Blurred => {
                self.editor_focused = false;
                cx.notify();
            }
        }
    }

    /// Validates the buffer after `delay`; a later call cancels this one.
    fn schedule_validation(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.validation = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((snapshot, known)) = this.update(cx, |view, cx| {
                (
                    view.editor.read(cx).snapshot(cx),
                    view.model.known().clone(),
                )
            }) else {
                return;
            };
            let mut result = cx
                .background_spawn(async move {
                    validate_text(snapshot.version(), snapshot.to_text(), &known)
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                let missing = std::mem::take(&mut result.missing);
                let model = &mut view.model;
                view.editor.update(cx, |editor, cx| {
                    model.accept(&mut editor.api(window, cx), result);
                });
                for gvk in view.model.to_fetch(&missing) {
                    view.fetch_schema(gvk, window, cx);
                }
                cx.notify();
            });
        }));
    }

    /// Fetches `gvk`'s schema off the UI thread and validates again when it arrives.
    fn fetch_schema(&mut self, gvk: Gvk, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.schemas.clone() else {
            // No cluster: syntax only.
            self.model
                .schema_arrived(gvk, Err(OxiError::not_found("no cluster")));
            return;
        };
        let (Some(port), Some(_)) = (source.port(cx), oxikube_runtime::mode(cx)) else {
            // Not connected yet: asked again on the next validation.
            self.model.forget_fetch(&gvk);
            return;
        };
        let cluster = source.cluster().clone();
        let wanted = gvk.clone();
        let fetch =
            oxikube_runtime::spawn_kube(
                cx,
                async move { port.schema_for(&cluster, &wanted).await },
            );
        self.schema_fetches
            .push(cx.spawn_in(window, async move |this, cx| {
                let outcome = fetch
                    .await
                    .unwrap_or_else(|error| Err(OxiError::internal(error.to_string())));
                let _ = this.update_in(cx, |view, window, cx| {
                    view.model.schema_arrived(gvk, outcome);
                    view.schedule_validation(Duration::ZERO, window, cx);
                });
            }));
    }
}

impl KeyContextual for ManifestEditor {
    const KEY_CONTEXT: &'static str = contexts::MANIFEST_EDITOR;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        context
            .value("mode", "yaml")
            .flag_if(self.editor_focused, contexts::EDITING);
    }
}

impl Render for ManifestEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        v_flex()
            .key_context(self.key_context())
            .on_action(cx.listener(|this, _: &ToggleReadOnly, _, cx| {
                this.send(Command::EditorToggleReadOnly, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSoftWrap, _, cx| {
                this.send(Command::EditorToggleSoftWrap, cx);
            }))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(self.toolbar(cx))
            .child(gpui::div().flex_1().min_h_0().child(self.editor.clone()))
    }
}
