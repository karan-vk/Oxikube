//! [`EditorAssist`]: hover docs and completion for a [`CodeEditor`](super::CodeEditor), in our
//! own types.
//!
//! A feature view (the manifest editor, E10-S05) implements the trait over what it knows about
//! the text (its schemas); `assist_bridge` plugs it into gpui-component's hover and completion
//! providers, so no feature crate names a library or language-server type. Offsets are UTF-8
//! byte offsets into the buffer, as everywhere in this module.
//!
//! Both calls run on the **background executor** with a [`TextSnapshot`] (the rope is shared, so
//! taking one on the UI thread is cheap); the bridge drops an answer for a cursor that has moved
//! on, so a slow answer never shows in the wrong place.

use std::ops::Range;

use super::code_editor::TextSnapshot;

/// A hover popup: Markdown `markdown` for the token at `range`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverInfo {
    /// The token the docs are about (the popup stays while the pointer is inside it).
    pub range: Range<usize>,
    /// The docs, as Markdown.
    pub markdown: String,
}

/// What a [`CompletionEntry`] inserts, for its icon and ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A field name (`image: `).
    Field,
    /// A value of the field being edited (an enum member, `true`).
    Value,
    /// A block of lines (a whole container).
    Snippet,
}

/// One line of the completion menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionEntry {
    /// What the menu shows and filters by.
    pub label: String,
    /// What it is.
    pub kind: EntryKind,
    /// A short note beside the label ("string, required").
    pub detail: Option<String>,
    /// The docs shown beside the menu for the selected entry, as Markdown.
    pub documentation: Option<String>,
    /// The text to put in place of [`replace`](Self::replace) (verbatim: no placeholders).
    pub insert_text: String,
    /// The bytes the entry replaces: the word being typed.
    pub replace: Range<usize>,
}

/// Hover and completion for one editor. `Send + Sync`: both calls run off the UI thread.
pub trait EditorAssist: Send + Sync + 'static {
    /// The docs for the token at `offset`, if there are any.
    fn hover(&self, text: &TextSnapshot, offset: usize) -> Option<HoverInfo>;

    /// The completions at the cursor `offset`, best first; empty when there is nothing to offer
    /// there (inside a comment, a block scalar, an open object). The library's menu shows them
    /// as they are: filter by the word before the cursor here.
    fn complete(&self, text: &TextSnapshot, offset: usize) -> Vec<CompletionEntry>;

    /// Whether typing `typed` (the text just inserted) asks for completions. The default is any
    /// one identifier character, so the menu follows the word being typed.
    fn triggers_completion(&self, typed: &str) -> bool {
        let mut chars = typed.chars();
        matches!(
            (chars.next(), chars.next()),
            (Some(c), None) if c.is_alphanumeric() || c == '_' || c == '-'
        )
    }
}
