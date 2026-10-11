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
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_settings::SettingsStore;
use oxikube_ui::ActiveTokens as _;
use oxikube_ui::editor::{CodeEditor, CodeEditorEvent, CodeEditorOptions, EditorApi as _};
use oxikube_ui::layout::v_flex;
use oxikube_workspace::{CommandDispatcher, ItemEvent};

use super::model::ManifestModel;
use super::notice::Notice;
use super::schemas::SchemaSource;
use super::validation::{VALIDATION_DEBOUNCE, validate_text_in};
use super::{
    ConvertToJson, ConvertToYaml, ToggleManagedFields, ToggleReadOnly, ToggleSecretMask,
    ToggleSoftWrap,
};
use crate::json::{Language, LanguageMode};
use crate::open::{RenderOptions, ResourceBinding, ResourceOpen};
use crate::settings::{EditorSettings, apply_changes};

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
    /// The live object to read and show (E10-S06); `None` for an editor on free text. The tab is
    /// then titled `kind/name` and `text` is ignored: the buffer fills when the object is read.
    pub resource: Option<ResourceOpen>,
}

/// The manifest editor: a YAML buffer with schema diagnostics, as a workspace item.
///
/// Every change restarts a [`VALIDATION_DEBOUNCE`] timer; the buffer is then validated on the
/// background executor and the result shown only if the buffer has not changed since. The
/// schemas its kinds need are fetched once, through the cluster's `SchemaPort` on the Kubernetes
/// runtime. Nothing blocks the UI thread.
pub struct ManifestEditor {
    pub(crate) editor: Entity<CodeEditor>,
    pub(crate) model: ManifestModel,
    pub(crate) title: SharedString,
    pub(crate) schemas: Option<Rc<dyn SchemaSource>>,
    dispatcher: Rc<dyn CommandDispatcher>,
    /// The buffer version that counts as unchanged (the text it opened with).
    pub(crate) clean_version: u64,
    /// The object this editor shows, when it was opened from one (E10-S06).
    pub(crate) binding: Option<ResourceBinding>,
    /// The `get` and render of the object in flight; replacing it cancels it.
    pub(crate) load_task: Option<Task<()>>,
    editor_focused: bool,
    /// The `editor` settings the buffer was last given (E10-S12): the next change is applied as
    /// the difference to this, so one setting changing leaves what another did to the editor
    /// alone. Never read to decide anything else: [`EditorSettings::resolve`] is the source.
    applied_settings: EditorSettings,
    /// What the notice bar says (a conversion's outcome); gone with the next edit.
    pub(super) notice: Option<Notice>,
    /// The next change event is the conversion's own: it keeps the notice.
    pub(super) keep_notice: bool,
    /// A running conversion; replacing it cancels the old one.
    pub(super) conversion: Option<Task<()>>,
    /// The pending or running validation; replacing it cancels the old one.
    validation: Option<Task<()>>,
    /// Schema fetches in flight; dropped (and aborted) with the editor.
    schema_fetches: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ManifestEditor {
    /// A new editor holding `parts.text`, validated at once.
    pub fn new(parts: ManifestEditorParts, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The content decides until the user (or a conversion) fixes the language; a tab opened
        // as JSON calls `use_language` right after creating the editor.
        let mode = LanguageMode::Auto;
        let language = mode.resolve(&parts.text);
        let options = match language {
            Language::Yaml => CodeEditorOptions::YAML,
            Language::Json => CodeEditorOptions::JSON,
        };
        let editor = cx.new(|cx| CodeEditor::new(options, window, cx));
        let cluster = parts
            .schemas
            .as_ref()
            .map(|source| source.cluster().clone());
        let settings = EditorSettings::resolve(cluster.as_ref(), cx).clone();
        editor.update(cx, |editor, cx| {
            apply_changes(None, &settings, &mut editor.api(window, cx));
        });
        if !parts.text.is_empty() {
            editor.update(cx, |editor, cx| {
                editor.api(window, cx).set_text(&parts.text)
            });
        }
        let clean_version = editor.read(cx).version();
        let mut binding = parts.resource.map(ResourceBinding::new);
        if let Some(binding) = binding.as_mut() {
            // `show_managed_fields` / `mask_secrets` are the defaults of a fresh open.
            let settings = EditorSettings::resolve(Some(&binding.target.cluster), cx);
            let options = RenderOptions {
                managed_fields: settings.show_managed_fields,
                reveal_secrets: !settings.mask_secrets,
            };
            binding.options = options;
            binding.shown_options = options;
        }
        let title = binding.as_ref().map_or(parts.title, ResourceBinding::title);
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            // Hot reload: dropped with the view, so a closed tab stops observing.
            cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
                this.settings_changed(window, cx);
            }),
        ];
        let mut model = ManifestModel::new();
        model.set_mode(mode);
        model.adopt(language);
        let mut this = Self {
            editor,
            model,
            title,
            schemas: parts.schemas,
            dispatcher: parts.dispatcher,
            clean_version,
            binding,
            load_task: None,
            editor_focused: false,
            applied_settings: settings,
            notice: None,
            keep_notice: false,
            conversion: None,
            validation: None,
            schema_fetches: Vec::new(),
            _subscriptions: subscriptions,
        };
        if this.binding.is_some() {
            // Nothing to type into until the object is read.
            this.editor
                .update(cx, |editor, cx| editor.api(window, cx).set_read_only(true));
            this.begin_load(window, cx);
        }
        this.schedule_validation(Duration::ZERO, window, cx);
        this
    }

    /// The code editor inside.
    pub fn editor(&self) -> &Entity<CodeEditor> {
        &self.editor
    }

    /// The object this editor was opened on (E10-S06), `None` for free text.
    pub fn resource_ref(&self) -> Option<&oxikube_domain::ids::ResourceRef> {
        self.binding.as_ref().map(ResourceBinding::target)
    }

    /// The object binding: how the object was opened and what the text shows of it.
    pub fn binding(&self) -> Option<&ResourceBinding> {
        self.binding.as_ref()
    }

    /// The view's state beyond the buffer (problems, schemas).
    pub fn model(&self) -> &ManifestModel {
        &self.model
    }

    /// The cluster whose schemas the buffer is checked against, and whose `clusters.<id>.editor`
    /// settings apply.
    pub fn cluster(&self) -> Option<&ClusterId> {
        self.schemas.as_deref().map(SchemaSource::cluster)
    }

    /// The `editor` settings in effect for this editor, read from the store now.
    pub fn settings<'a>(&self, cx: &'a gpui::App) -> &'a EditorSettings {
        EditorSettings::resolve(self.cluster(), cx)
    }

    /// The language the buffer is read as now (see [`LanguageMode`]).
    pub fn language(&self) -> Language {
        self.model.language()
    }

    /// What the notice bar says now, if anything.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Shows `notice` in the bar.
    pub(super) fn set_notice(&mut self, notice: Notice, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
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

    /// The settings store changed: applies what differs for this editor, at once. The buffer, its
    /// selections and its undo history are untouched. Turning `validation` off clears the
    /// squiggles; turning it on checks the buffer again.
    fn settings_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.settings(cx);
        if *next == self.applied_settings {
            return;
        }
        let next = next.clone();
        let previous = std::mem::replace(&mut self.applied_settings, next);
        let next = &self.applied_settings;
        self.editor.update(cx, |editor, cx| {
            apply_changes(Some(&previous), next, &mut editor.api(window, cx));
        });
        if previous.validation != next.validation {
            self.schedule_validation(Duration::ZERO, window, cx);
        }
        cx.notify();
    }

    /// Sends `command` to the bus (the keys, the toolbar and the palette all end up here).
    pub(crate) fn send(&self, command: Command, cx: &mut Context<Self>) {
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
                // The conversion's own change keeps its notice; any other edit (an undo too)
                // makes it stale.
                if !std::mem::take(&mut self.keep_notice) {
                    self.notice = None;
                }
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
    pub(crate) fn schedule_validation(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings(cx).validation {
            // Off: no squiggles, no work, no schema fetches.
            self.validation = None;
            let model = &mut self.model;
            self.editor.update(cx, |editor, cx| {
                model.clear(&mut editor.api(window, cx));
            });
            return;
        }
        self.validation = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((snapshot, known, mode, masking)) = this.update(cx, |view, cx| {
                (
                    view.editor.read(cx).snapshot(cx),
                    view.model.known().clone(),
                    view.model.mode(),
                    view.binding.as_ref().is_some_and(ResourceBinding::masking),
                )
            }) else {
                return;
            };
            let mut result = cx
                .background_spawn(async move {
                    validate_text_in(
                        snapshot.version(),
                        snapshot.to_text(),
                        &known,
                        mode,
                        masking,
                    )
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
            .value("mode", self.model.language().grammar())
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
            .on_action(cx.listener(|this, _: &ToggleManagedFields, _, cx| {
                this.send(Command::EditorToggleManagedFields, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSecretMask, _, cx| {
                this.send(Command::EditorToggleSecretMask, cx);
            }))
            .on_action(cx.listener(|this, _: &ConvertToJson, _, cx| {
                this.send(Command::EditorConvertToJson, cx);
            }))
            .on_action(cx.listener(|this, _: &ConvertToYaml, _, cx| {
                this.send(Command::EditorConvertToYaml, cx);
            }))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(self.toolbar(cx))
            .children(self.notice_bar(cx))
            .child(gpui::div().flex_1().min_h_0().child(self.editor.clone()))
    }
}
