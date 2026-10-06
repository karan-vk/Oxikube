//! The code view: gpui-component's editor, read-only, with tree-sitter highlighting.
//!
//! The resource detail's YAML tab shows an object this way, and its Describe tab shows text the
//! same way without a language. The editor is the library's (rope text, tree-sitter parse, line
//! numbers, search, selection and copy); this module fixes the settings every read-only view
//! wants and keeps the library out of the feature crates. Editing (apply, validation, diff) is
//! the manifest editor's job (`oxikube_editor`, E10).
//!
//! The text is set with [`set_text`], which re-parses once; nothing here runs per frame.

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
