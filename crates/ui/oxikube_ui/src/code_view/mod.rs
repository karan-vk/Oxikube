//! [`CodeView`]: a read-only, virtualised view of a text of any size, with tree-sitter colours,
//! line numbers, soft wrap, selection and copy (E07-P598).
//!
//! The resource detail's YAML and Describe tabs show an object this way. A multi-megabyte object
//! is too big for the full editor ([`crate::editor`]), which wraps and measures every line on the
//! UI thread when it is given its text; this view does no per-line work on the UI thread at all:
//!
//! | File | Holds |
//! |---|---|
//! | `rows` | [`RowMap`]: where each display row starts and ends (soft wrap at the view's width, or a long line cut at [`MAX_ROW_COLS`]); pure, built off the UI thread |
//! | `highlight` | the tree-sitter parse (off the UI thread) and the styles of the rows on screen, cached while they stay on screen |
//! | `view` | [`CodeView`], [`Look`]: the text given, the layout and parse sent to the background executor, the scroll |
//! | `selection` | click, shift-click, drag, double- and triple-click; copy and select all; key scrolling |
//! | `render` | the `uniform_list` of the visible rows, the gutter, the scrollbar, the probe that measures the width |
//!
//! # What runs where
//!
//! [`CodeView::set_text`] takes the text as an `Arc<str>` (no copy) and lays its rows out on the
//! background executor; the previous text stays on screen until the new rows land, and the first
//! text shows as soon as its rows are ready, coloured once its parse lands. A frame slices,
//! colours and shapes only the rows in the viewport (one run each, bounded by the row width), so
//! its cost does not grow with the text. A new width re-wraps off the UI thread too. The font is
//! monospace, so a column is one advance: rows have one height and a pointer position maps to a
//! byte without the shaped text.
//!
//! # Keys
//!
//! The view has the `CodeView` key context while focused (a click focuses it): `cmd-c` / `ctrl-c`
//! copies the selection, `cmd-a` / `ctrl-a` selects all, the arrows, page keys and `cmd-up` /
//! `cmd-down` (`ctrl-home` / `ctrl-end`) scroll. These are the view's own editing keys, bound by
//! [`crate::init`] like the component library binds its inputs' keys; every other key reaches the
//! enclosing view.

mod highlight;
mod render;
mod rows;
mod selection;
mod view;

#[cfg(test)]
mod tests;

use gpui::{App, KeyBinding, Pixels, actions, px};

pub use rows::{MAX_ROW_COLS, Row, RowMap};
pub use view::{CodeView, Look};

/// The key context a focused [`CodeView`] sets.
pub const KEY_CONTEXT: &str = "CodeView";

/// The height of a row, at 100 % zoom.
pub const ROW_HEIGHT: Pixels = px(20.);

/// Room kept right of wrapped text for the scrollbar.
pub(crate) const SCROLLBAR_GAP: Pixels = px(12.);

actions!(
    code_view,
    [
        /// Copy the selected text.
        Copy,
        /// Select the whole text.
        SelectAll,
        /// Scroll one row up.
        LineUp,
        /// Scroll one row down.
        LineDown,
        /// Scroll one screen up.
        PageUp,
        /// Scroll one screen down.
        PageDown,
        /// Scroll to the first row.
        Top,
        /// Scroll to the last row.
        Bottom,
    ]
);

/// Binds the view's keys in the [`KEY_CONTEXT`] context. Called by [`crate::init`].
pub(crate) fn init(cx: &mut App) {
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("up", LineUp, context),
        KeyBinding::new("down", LineDown, context),
        KeyBinding::new("pageup", PageUp, context),
        KeyBinding::new("pagedown", PageDown, context),
        KeyBinding::new("secondary-up", Top, context),
        KeyBinding::new("secondary-down", Bottom, context),
        KeyBinding::new("ctrl-home", Top, context),
        KeyBinding::new("ctrl-end", Bottom, context),
    ]);
}
