//! [`CodeEditor`]: gpui-component's editor set up for editing code, with the diagnostics overlay,
//! behind [`EditorApi`] ([`LiveEditor`]).
//!
//! What the library brings is kept as it is: rope text, the tree-sitter parse and highlighting,
//! line numbers, folding (the gutter chevrons), search and replace (`cmd-f` / `ctrl-f`, with
//! `cmd-shift-f` / `ctrl-h` for replace), multiple cursors (`cmd-alt-up/down`, `shift-alt-up/down`
//! on Linux), undo and redo, soft wrap and read-only. This module adds a version counter for
//! stale-result checks, our diagnostics (squiggles through the library's `DiagnosticSet`, plus
//! the gutter markers and end-of-line messages of [`super::overlay`]) and decorations.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
};
use gpui_component::input::{
    Editor, EditorState, InputEvent, RangeDecorationCollection, Rope, TextDecorationCollection,
};

#[cfg(doc)]
use super::api::EditorApi;
use super::api::EditorDiagnostic;
use super::live::LiveEditor;
use super::overlay::{diagnostics_overlay, status_label};
use super::positions::LineSummary;
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
}

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
    rope: Rope,
    version: u64,
}

impl TextSnapshot {
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
    pub(super) diagnostics: Vec<EditorDiagnostic>,
    pub(super) summaries: Rc<Vec<LineSummary>>,
    pub(super) status: SharedString,
    pub(super) dimmed: Option<TextDecorationCollection>,
    pub(super) fills: Option<RangeDecorationCollection>,
    pub(super) frames: Option<RangeDecorationCollection>,
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
            diagnostics: Vec::new(),
            summaries: Rc::default(),
            status: status_label(&[], 0, 0).into(),
            dimmed: None,
            fills: None,
            frames: None,
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

    /// The buffer now, to turn into text off the UI thread.
    pub fn snapshot(&self, cx: &gpui::App) -> TextSnapshot {
        TextSnapshot {
            rope: self.state.read(cx).text().clone(),
            version: self.version,
        }
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
        let tokens = cx.tokens();
        div()
            .relative()
            .size_full()
            .child(
                Editor::new(&self.state)
                    // The element re-applies these to the state on every render.
                    .readonly(self.read_only)
                    .appearance(false)
                    .bordered(false)
                    .text_size(u(tokens.font.mono))
                    .size_full(),
            )
            .child(diagnostics_overlay(
                self.state.clone(),
                self.summaries.clone(),
                self.status.clone(),
            ))
    }
}
