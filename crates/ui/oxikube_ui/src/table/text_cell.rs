//! [`TextCell`]: the fast path for a cell that is one line of text in one colour (E07-S09).
//!
//! A cell drawn from [`TableDelegate::render_td`](super::TableDelegate::render_td) is the
//! delegate's element inside the alignment box, and a cell that should end in an ellipsis when it
//! is too narrow carries GPUI's `text_ellipsis`. That style is the expensive part of a table frame:
//! a truncating text cannot reuse its measured size, so every layout pass over it shapes the text
//! again and asks the text system for a line wrapper. Most cells fit their column, so
//! [`text_cell`] measures the text once against the column (through the text system's line layout
//! cache, which keeps the previous frame's lines) and only a cell that does not fit gets the
//! ellipsis box; one that fits is a bare styled text, one element less and measured once.
//!
//! The decision keeps a small margin ([`FIT_SLACK`]), so a text that fits only by a fraction of a
//! pixel takes the ellipsis path and is drawn exactly as before: either way it shows the same
//! glyphs.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, HighlightStyle, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Pixels, SharedString, Styled as _, StyledText, Window, div, px,
};

use super::column::ColumnAlign;
use crate::size::ControlSize;

/// A cell that is one line of text, optionally coloured. Returned by
/// [`TableDelegate::text_cell`](super::TableDelegate::text_cell).
#[derive(Clone, Debug, PartialEq)]
pub struct TextCell {
    /// The text (cheap to clone: keep these cached between frames where you can).
    pub text: SharedString,
    /// Its colour; `None` inherits the table's text colour.
    pub color: Option<Hsla>,
}

impl TextCell {
    /// `text` in the table's text colour.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            color: None,
        }
    }

    /// The same text in `color`.
    #[must_use]
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

/// How much narrower than the column a text must be to be drawn without the ellipsis box.
pub(super) const FIT_SLACK: Pixels = px(1.);

/// The horizontal padding the library puts inside every body cell (left plus right). The table
/// always hands the library a `ControlSize::Size` (see `element`), whose padding does not depend on
/// the height it carries.
fn cell_padding_x() -> Pixels {
    let padding = ControlSize::Size(px(0.)).table_cell_padding();
    padding.left + padding.right
}

/// The width `text` takes in the current text style, from the line layout cache.
fn text_width(text: &SharedString, window: &Window) -> Pixels {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let run = style.to_run(text.len());
    window
        .text_system()
        .layout_line(text, font_size, &[run], None)
        .width
}

/// Whether `cell` fits a column `column_width` wide (on screen, padding included).
pub(super) fn fits(cell: &TextCell, column_width: Pixels, window: &Window) -> bool {
    let room = column_width - cell_padding_x() - FIT_SLACK;
    room > Pixels::ZERO && text_width(&cell.text, window) <= room
}

/// The element of a text cell at (`row`, `col`) in a column `column_width` wide (unknown before
/// the library first read the column: the ellipsis path, which is always correct). Tagged
/// `td-<row>-<col>` for test bounds.
pub(super) fn text_cell(
    cell: TextCell,
    align: ColumnAlign,
    column_width: Option<Pixels>,
    row: usize,
    col: usize,
    window: &Window,
) -> AnyElement {
    let fitting = column_width.is_some_and(|width| fits(&cell, width, window));
    let body = if fitting {
        let len = cell.text.len();
        let highlight = cell.color.map(|color| {
            (
                0..len,
                HighlightStyle {
                    color: Some(color),
                    ..HighlightStyle::default()
                },
            )
        });
        StyledText::new(cell.text)
            .with_highlights(highlight)
            .into_any_element()
    } else {
        div()
            .w_full()
            .text_ellipsis()
            .when_some(cell.color, |d, color| d.text_color(color))
            .child(cell.text)
            .into_any_element()
    };
    super::adapter::aligned(align, body)
        .debug_selector(move || format!("td-{row}-{col}"))
        .into_any_element()
}
