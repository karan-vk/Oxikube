//! The code editors: gpui-component's editor behind our own API.
//!
//! - [`CodeEditor`] (`code_editor`): the editable editor the manifest editor (`oxikube_editor`,
//!   E10-S04) is built on: line numbers, folding, search and replace, multiple cursors, undo,
//!   read-only and soft wrap from the library; a version counter, diagnostics (squiggles, gutter
//!   markers, end-of-line messages) and decorations from us.
//! - [`EditorApi`] (`api`): the trait feature views drive it through, in byte offsets and our own
//!   types ([`EditorDiagnostic`], [`Decoration`]), so no feature crate imports gpui-component.
//!   [`LiveEditor`] implements it over a [`CodeEditor`]; tests implement it with a plain struct.
//! - `positions`: the one place byte offsets become the library's line/character positions.
//! - `overlay`: the gutter markers and end-of-line messages, painted in a canvas over the visible
//!   lines (the library has no inline widgets).
//! - The read-only setup below ([`read_only_state`], [`code_view`]): what a view that only shows
//!   text wants.
//!
//! The text is set with [`set_text`] / [`EditorApi::set_text`], which re-parse once; nothing here
//! runs per frame beyond painting the visible lines.

mod api;
mod code_editor;
mod live;
mod overlay;
mod positions;

pub use api::{Decoration, DecorationStyle, DiagnosticLevel, EditorApi, EditorDiagnostic};
pub use code_editor::{CodeEditor, CodeEditorEvent, CodeEditorOptions, TextSnapshot};
pub use live::LiveEditor;

use gpui::{App, AppContext as _, Entity, Pixels, Point, Window};
pub use gpui_component::input::{Editor, EditorState};

/// How a read-only view looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOnly {
    /// The tree-sitter grammar to highlight with (`"yaml"`, `"json"`); `None` shows plain text.
    pub language: Option<&'static str>,
    /// Number the lines.
    pub line_numbers: bool,
    /// Wrap long lines at the view's width instead of scrolling sideways.
    pub soft_wrap: bool,
}

impl ReadOnly {
    /// Highlighted YAML with line numbers; long lines wrap.
    pub const YAML: ReadOnly = ReadOnly {
        language: Some("yaml"),
        line_numbers: true,
        soft_wrap: true,
    };

    /// Plain text (`kubectl describe` output): no line numbers, columns stay aligned (long lines
    /// scroll sideways).
    pub const TEXT: ReadOnly = ReadOnly {
        language: None,
        line_numbers: false,
        soft_wrap: false,
    };
}

/// A new read-only editor state looking as `look` says: no editing. Create it once and keep it;
/// feed it text with [`set_text`].
pub fn read_only_state(look: ReadOnly, window: &mut Window, cx: &mut App) -> Entity<EditorState> {
    cx.new(|cx| {
        let mut state = EditorState::new(window, cx);
        if let Some(language) = look.language {
            state = state.language(language);
        }
        state.set_readonly(true, cx);
        state.set_line_number(look.line_numbers, window, cx);
        state.set_soft_wrap(look.soft_wrap, window, cx);
        state
    })
}

/// Replaces the text of `state`: the highlighter re-parses once, the undo history is cleared and
/// no change event is emitted. The scroll position stays where the reader left it (the view
/// follows an object that changes under it without jumping to the top).
pub fn set_text(state: &Entity<EditorState>, text: &str, window: &mut Window, cx: &mut App) {
    let text = text.to_owned();
    state.update(cx, |state, cx| {
        let offset: Point<Pixels> = state.scroll_offset();
        state.set_value(text, window, cx);
        state.set_scroll_offset(offset, cx);
    });
}

/// The text `state` holds.
pub fn text(state: &Entity<EditorState>, cx: &App) -> String {
    state.read(cx).value().to_string()
}

/// The element drawing `state`: read-only, without the input chrome. Give it a size through its
/// parent.
pub fn code_view(state: &Entity<EditorState>) -> Editor {
    Editor::new(state)
        .readonly(true)
        .appearance(false)
        .bordered(false)
}
