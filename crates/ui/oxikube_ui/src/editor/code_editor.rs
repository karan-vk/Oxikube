//! [`CodeEditor`]: gpui-component's editor set up for editing code, with the diagnostics overlay,
//! behind [`EditorApi`] ([`LiveEditor`]).
//!
//! What the library brings is kept as it is: rope text, the tree-sitter parse and highlighting,
//! line numbers, folding (the gutter chevrons), search and replace (`cmd-f` / `ctrl-f`, with
//! `cmd-shift-f` / `ctrl-h` for replace), multiple cursors (`cmd-alt-up/down`, `shift-alt-up/down`
//! on Linux), undo and redo, soft wrap and read-only. This module adds a version counter for
//! stale-result checks, our diagnostics (squiggles through the library's `DiagnosticSet`, plus
//! the gutter markers and end-of-line messages of [`super::overlay`]) and decorations.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, Task, Window, div, px,
};
use gpui_component::input::{
    Editor, EditorState, Enter, Escape, InputEvent, MoveDown, MoveUp, RangeDecorationCollection,
    Rope, TextDecorationCollection,
};

#[cfg(doc)]
use super::api::EditorApi;
use super::api::{EditorDiagnostic, EditorFont};
use super::assist::EditorAssist;
use super::live::LiveEditor;
use super::overlay::{OverlayStats, diagnostics_overlay, status_label};
use super::positions::{LineSummary, rows_to_bytes};
use crate::size::u;
use crate::tokens::ActiveTokens as _;

/// How a [`CodeEditor`] starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeEditorOptions {
    /// The tree-sitter grammar (`"yaml"`, `"json"`).
    pub language: &'static str,
    /// Number the lines.
    pub line_numbers: bool,
    /// Wrap long lines at the view's width.
    pub soft_wrap: bool,
    /// Refuse typing.
    pub read_only: bool,
}

impl CodeEditorOptions {
    /// Editable YAML with line numbers; long lines scroll sideways.
    pub const YAML: CodeEditorOptions = CodeEditorOptions {
        language: "yaml",
        line_numbers: true,
        soft_wrap: false,
        read_only: false,
    };

    /// Editable JSON (tree-sitter-json) with line numbers; long lines scroll sideways.
    pub const JSON: CodeEditorOptions = CodeEditorOptions {
        language: "json",
        ..CodeEditorOptions::YAML
    };
}

/// The indent width a new editor starts with (the library's own default).
pub const DEFAULT_TAB_SIZE: usize = 2;

/// What a [`CodeEditor`] tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeEditorEvent {
    /// The buffer changed; `version` is its new [`EditorApi::version`].
    Changed {
        /// The new version.
        version: u64,
    },
    /// The buffer took the keyboard focus.
    Focused,
    /// The buffer lost the keyboard focus.
    Blurred,
}

/// The buffer at one version, cheap to take on the UI thread (the rope is shared) and turned
/// into a string off it.
#[derive(Clone)]
pub struct TextSnapshot {
    pub(super) rope: Rope,
    version: u64,
}

impl TextSnapshot {
    /// A snapshot of `rope` (the library's text) at `version`.
    pub(super) fn of(rope: Rope, version: u64) -> Self {
        Self { rope, version }
    }

    /// Whether both snapshots hold the same text, whatever their versions. Compares the text
    /// (memory speed), so call it off the UI thread for a large buffer.
    pub fn same_text(&self, other: &TextSnapshot) -> bool {
        self.rope == other.rope
    }

    /// The bytes `from..to` of the text (clamped to the buffer).
    pub(super) fn text_between(&self, from: usize, to: usize) -> String {
        let len = self.rope.len();
        let to = to.min(len);
        self.rope.slice(from.min(to)..to).to_string()
    }

    /// The version the snapshot was taken at.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The text. Allocates the whole buffer: call it on a background thread for large buffers.
    pub fn to_text(&self) -> Arc<str> {
        Arc::from(self.rope.to_string())
    }
}

/// gpui-component's editor with our version counter, diagnostics overlay and decorations. Drive
/// it through [`CodeEditor::api`]; render it as a child that fills its parent.
pub struct CodeEditor {
    pub(super) state: Entity<EditorState>,
    pub(super) version: u64,
    pub(super) read_only: bool,
    pub(super) soft_wrap: bool,
    pub(super) tab_size: usize,
    pub(super) font: EditorFont,
    pub(super) language: &'static str,
    pub(super) diagnostics: Vec<EditorDiagnostic>,
    pub(super) summaries: Rc<Vec<LineSummary>>,
    pub(super) status: SharedString,
    pub(super) dimmed: Option<TextDecorationCollection>,
    pub(super) fills: Option<RangeDecorationCollection>,
    pub(super) frames: Option<RangeDecorationCollection>,
    overlay_stats: Rc<Cell<OverlayStats>>,
    /// Hover and completion (`set_assist`), if the owner gave the editor any.
    pub(super) assist: Option<Arc<dyn EditorAssist>>,
    /// The keyboard-opened completion or hover being computed; replacing it cancels the old one.
    pub(super) assist_task: Task<()>,
    _events: Subscription,
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

impl CodeEditor {
    /// A new, empty editor.
    pub fn new(options: CodeEditorOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(options.language)
                .searchable(true);
            state.set_readonly(options.read_only, cx);
            state.set_line_number(options.line_numbers, window, cx);
            state.set_soft_wrap(options.soft_wrap, window, cx);
            state
        });
        let events =
            cx.subscribe_in(
                &state,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.changed(cx),
                    InputEvent::Focus => cx.emit(CodeEditorEvent::Focused),
                    InputEvent::Blur => cx.emit(CodeEditorEvent::Blurred),
                    InputEvent::PressEnter { .. } => {}
                },
            );
        Self {
            state,
            version: 0,
            read_only: options.read_only,
            soft_wrap: options.soft_wrap,
            tab_size: DEFAULT_TAB_SIZE,
            font: EditorFont::default(),
            language: options.language,
            diagnostics: Vec::new(),
            summaries: Rc::default(),
            status: status_label(&[], 0, 0).into(),
            dimmed: None,
            fills: None,
            frames: None,
            overlay_stats: Rc::default(),
            assist: None,
            assist_task: Task::ready(()),
            _events: events,
        }
    }

    /// The library state (for this crate's tests and helpers only).
    #[doc(hidden)]
    pub fn state(&self) -> &Entity<EditorState> {
        &self.state
    }

    /// The buffer's version (see [`EditorApi::version`]).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Whether typing is refused (see [`EditorApi::is_read_only`]).
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether long lines wrap (see [`EditorApi::soft_wrap`]).
    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// The indent width (see [`EditorApi::tab_size`]).
    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    /// The font settings (see [`EditorApi::font`]).
    pub fn font(&self) -> &EditorFont {
        &self.font
    }

    /// The size the text is drawn at now, in pixels: the font size (or the theme's monospace
    /// size) times the UI zoom.
    pub fn text_size(&self, cx: &gpui::App) -> Pixels {
        u(self.font_size(cx))
    }

    /// The font size before the UI zoom.
    fn font_size(&self, cx: &gpui::App) -> Pixels {
        self.font.size.map_or_else(|| cx.tokens().font.mono, px)
    }

    /// The grammar the buffer is highlighted with (see [`EditorApi::language`]).
    pub fn language(&self) -> &'static str {
        self.language
    }

    /// Undoes the last edit of the buffer, as the undo key does: focuses the buffer and sends it
    /// the library's undo action. For a button that offers to take an edit back.
    pub fn undo(&self, window: &mut Window, cx: &mut gpui::App) {
        use gpui::Action as _;
        window.focus(&self.focus_handle(cx), cx);
        window.dispatch_action(gpui_component::input::Undo.boxed_clone(), cx);
    }

    /// The buffer now, to turn into text off the UI thread.
    pub fn snapshot(&self, cx: &gpui::App) -> TextSnapshot {
        TextSnapshot {
            rope: self.state.read(cx).text().clone(),
            version: self.version,
        }
    }

    /// The buffer's length in bytes (cheap: the rope keeps it).
    pub fn len(&self, cx: &gpui::App) -> usize {
        self.state.read(cx).text().len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self, cx: &gpui::App) -> bool {
        self.len(cx) == 0
    }

    /// The bytes of the buffer lines laid out in the last frame (the visible ones, plus the few
    /// the library lays out around them), `None` before the first frame. For work that should
    /// start with what the reader sees.
    pub fn visible_range(&self, cx: &gpui::App) -> Option<Range<usize>> {
        let state = self.state.read(cx);
        let rows = state.visible_row_range()?;
        Some(rows_to_bytes(state.text(), rows))
    }

    /// What the diagnostics overlay drew in the last frame: lines painted, out of the lines with
    /// a diagnostic. The overlay looks at the visible lines only, so `painted` stays at a
    /// screenful however many findings the buffer has.
    pub fn overlay_stats(&self) -> OverlayStats {
        self.overlay_stats.get()
    }

    /// What the overlay's status element says ("2 errors, 1 warning").
    pub fn status(&self) -> &SharedString {
        &self.status
    }

    /// The editor behind [`EditorApi`], for the length of one borrow.
    pub fn api<'a, 'b>(
        &'a mut self,
        window: &'a mut Window,
        cx: &'a mut Context<'b, Self>,
    ) -> LiveEditor<'a, 'b> {
        LiveEditor::new(self, window, cx)
    }

    pub(super) fn changed(&mut self, cx: &mut Context<Self>) {
        self.version += 1;
        // The library drops its squiggles on every edit; the overlay follows it.
        self.clear_overlay();
        cx.emit(CodeEditorEvent::Changed {
            version: self.version,
        });
        cx.notify();
    }

    fn clear_overlay(&mut self) {
        if !self.diagnostics.is_empty() {
            self.diagnostics.clear();
            self.summaries = Rc::default();
            self.status = status_label(&[], 0, 0).into();
        }
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            // The completion menu's keys, before the editor's own (see `route_to_menu`).
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                this.route_to_menu(action, window, cx);
            }))
            .capture_action(cx.listener(|this, action: &Escape, window, cx| {
                this.route_to_menu(action, window, cx);
            }))
            .capture_action(cx.listener(|this, action: &MoveUp, window, cx| {
                this.route_to_menu(action, window, cx);
            }))
            .capture_action(cx.listener(|this, action: &MoveDown, window, cx| {
                this.route_to_menu(action, window, cx);
            }))
            .child(
                Editor::new(&self.state)
                    // The element re-applies these to the state on every render.
                    .readonly(self.read_only)
                    .appearance(false)
                    .bordered(false)
                    .text_size(u(self.font_size(cx)))
                    .when_some(self.font.family.clone(), |editor, family| {
                        editor.font_family(SharedString::from(family))
                    })
                    .size_full(),
            )
            .child(diagnostics_overlay(
                self.state.clone(),
                self.summaries.clone(),
                self.status.clone(),
                self.overlay_stats.clone(),
            ))
    }
}
