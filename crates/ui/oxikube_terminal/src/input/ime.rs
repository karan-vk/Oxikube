//! IME composition: [`EntityInputHandler`] for [`TerminalState`].
//!
//! While an input method composes (a Japanese or Chinese reading, a dead key, a Hangul syllable)
//! the platform hands us *marked text*. It is not sent to the process: the element paints it
//! inline at the cursor, underlined, and the candidate window is placed beside it
//! ([`bounds_for_range`](EntityInputHandler::bounds_for_range)). When the user commits,
//! `replace_text_in_range` arrives with the final text and it is written to the process as UTF-8.
//! Plain typing (no IME) takes the same path: the key-down listener leaves text keys alone and the
//! platform delivers them here.
//!
//! The element keeps the [`ImeAnchor`] (cell size, cursor cell) up to date each frame; nothing
//! here reads the grid.

use std::ops::Range;

use gpui::{
    Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, Point, Size, UTF16Selection,
    Window, point, size,
};

use crate::state::TerminalState;

/// Where the cursor is on screen, for the candidate window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImeAnchor {
    /// The size of one cell.
    pub cell: Size<Pixels>,
    /// The cursor's viewport row.
    pub row: usize,
    /// The cursor's viewport column.
    pub column: usize,
}

/// The text being composed and where the input method's caret is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    /// The marked text.
    pub text: String,
    /// The input method's selection inside `text`, in UTF-16 units.
    pub selected: Range<usize>,
}

/// What a terminal keeps for its input method. Never logged: it is what the user is typing.
#[derive(Default)]
pub(crate) struct ImeState {
    pub(crate) composition: Option<Composition>,
    pub(crate) anchor: Option<ImeAnchor>,
}

impl TerminalState {
    /// The text being composed, if an input method is composing.
    pub fn composition(&self) -> Option<&Composition> {
        self.ime.composition.as_ref()
    }

    /// Whether an input method is composing: keys then belong to it.
    pub fn is_composing(&self) -> bool {
        self.ime.composition.is_some()
    }

    /// Tells the state where the cursor is on screen (the element does, each frame). Does not
    /// notify: nothing about it is painted.
    pub fn set_ime_anchor(&mut self, anchor: ImeAnchor) {
        self.ime.anchor = Some(anchor);
    }

    /// The last [`ImeAnchor`].
    pub fn ime_anchor(&self) -> Option<ImeAnchor> {
        self.ime.anchor
    }
}

/// The cells `c` takes: 0 for combining marks, 2 for the East Asian wide ranges, else 1.
fn cells_of(c: char) -> usize {
    match u32::from(c) {
        0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F => 0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0xA4CF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// The cells `text` takes.
pub fn cells_in(text: &str) -> usize {
    text.chars().map(cells_of).sum()
}

/// `text` between UTF-16 offsets `range`, clamped to char boundaries.
fn slice_utf16(text: &str, range: &Range<usize>) -> String {
    let mut offset = 0;
    let mut out = String::new();
    for c in text.chars() {
        let next = offset + c.len_utf16();
        if offset >= range.start && next <= range.end {
            out.push(c);
        }
        offset = next;
    }
    out
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl EntityInputHandler for TerminalState {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let composition = self.ime.composition.as_ref()?;
        let end = range.end.min(utf16_len(&composition.text));
        let start = range.start.min(end);
        *adjusted_range = Some(start..end);
        Some(slice_utf16(&composition.text, &(start..end)))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // A terminal has no editable text of its own: the caret is the composition's, or nowhere.
        let range = self
            .ime
            .composition
            .as_ref()
            .map_or(0..0, |composition| composition.selected.clone());
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.ime
            .composition
            .as_ref()
            .map(|composition| 0..utf16_len(&composition.text))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.ime.composition.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The commit: the composition ends and the final text goes to the process.
        if self.ime.composition.take().is_some() {
            cx.notify();
        }
        if !text.is_empty() {
            self.type_text(text, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime.composition = (!new_text.is_empty()).then(|| {
            let length = utf16_len(new_text);
            Composition {
                text: new_text.to_owned(),
                selected: new_selected_range
                    .map(|range| range.start.min(length)..range.end.min(length))
                    .unwrap_or(length..length),
            }
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let anchor = self.ime.anchor?;
        let (before, within) = match &self.ime.composition {
            Some(composition) => (
                cells_in(&slice_utf16(
                    &composition.text,
                    &(0..range_utf16.start.min(utf16_len(&composition.text))),
                )),
                cells_in(&slice_utf16(&composition.text, &range_utf16)),
            ),
            None => (0, 0),
        };
        let origin: Point<Pixels> = point(
            element_bounds.origin.x + anchor.cell.width * (anchor.column + before) as f32,
            element_bounds.origin.y + anchor.cell.height * anchor.row as f32,
        );
        Some(Bounds::new(
            origin,
            size(anchor.cell.width * within.max(1) as f32, anchor.cell.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    /// A platform paste (the Edit menu, a Wayland paste request) goes through `terminal::Paste`
    /// like the keymap's, so bracketed paste and the multi-line confirmation apply.
    fn paste(&mut self, _: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(Box::new(super::Paste), cx);
    }
}
