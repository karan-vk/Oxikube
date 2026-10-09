//! Selecting text with the pointer (click, shift-click, drag, double-click a word, triple-click a
//! line), copying it, and scrolling with the keys. A position maps to a byte through the row grid
//! (uniform rows, monospace columns), so nothing here needs the shaped text.

use std::ops::Range;

use gpui::{ClipboardItem, Context, MouseDownEvent, Pixels, Point, Window, px};

use super::CodeView;
use super::rows::offset_at_col;

/// The selected bytes: from `anchor` (where the press was) to `head` (where the pointer is).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Selection {
    pub(super) anchor: usize,
    pub(super) head: usize,
    /// A press is held: pointer moves extend the selection.
    pub(super) dragging: bool,
}

impl Selection {
    /// The selected bytes, when any are.
    pub(super) fn range(&self) -> Option<Range<usize>> {
        (self.anchor != self.head).then(|| self.anchor.min(self.head)..self.anchor.max(self.head))
    }
}

/// Whether `ch` belongs to a word a double-click selects (names, numbers, keys like `app.kubernetes.io/name`).
fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')
}

impl CodeView {
    /// The selected text.
    pub fn selected_text(&self) -> Option<&str> {
        let range = self.selection.range()?;
        self.shown.as_ref()?.text.get(range)
    }

    /// Selects the whole text (`code_view::SelectAll`).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.shown.as_ref() else {
            return;
        };
        self.selection = Selection {
            anchor: 0,
            head: shown.text.len(),
            dragging: false,
        };
        cx.notify();
    }

    /// Copies the selection to the clipboard (`code_view::Copy`); nothing without one.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    /// The byte under `position`, from the last frame's bounds and scroll.
    pub(super) fn offset_at(&self, position: Point<Pixels>) -> Option<usize> {
        let shown = self.shown.as_ref()?;
        let bounds = self.metrics.bounds?;
        let row_height = self.metrics.row_height;
        if row_height <= px(0.) || self.metrics.advance <= px(0.) {
            return None;
        }
        let scroll = self.scroll_offset();
        let y = position.y - bounds.top() - scroll.y;
        let ix = if y <= px(0.) {
            0
        } else {
            ((y / row_height).floor() as usize).min(shown.rows.len().saturating_sub(1))
        };
        let row = shown.rows.row(ix)?;
        let text_left =
            self.metrics.advance * shown.rows.gutter_cols() as f32 + self.metrics.padding;
        let x = position.x - bounds.left() - scroll.x - text_left;
        let col = if x <= px(0.) {
            0
        } else {
            (x / self.metrics.advance).round() as usize
        };
        let row_text = shown.text.get(row.start..row.end)?;
        Some(row.start + offset_at_col(row_text, col))
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let Some(offset) = self.offset_at(event.position) else {
            return;
        };
        let text = match self.shown.as_ref() {
            Some(shown) => shown.text.clone(),
            None => return,
        };
        let (anchor, head) = match event.click_count {
            2 => word_at(&text, offset),
            n if n >= 3 => line_at(&text, offset),
            _ if event.modifiers.shift => (self.selection.anchor, offset),
            _ => (offset, offset),
        };
        self.selection = Selection {
            anchor,
            head,
            dragging: true,
        };
        cx.notify();
    }

    /// The pointer moved with the button held: extend the selection, scrolling when it leaves the
    /// view above or below.
    pub(super) fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selection.dragging {
            return;
        }
        let Some(bounds) = self.metrics.bounds else {
            return;
        };
        let mut inside = position;
        if position.y < bounds.top() {
            self.set_scroll_y(self.scroll_offset().y + (bounds.top() - position.y));
            inside.y = bounds.top();
        } else if position.y > bounds.bottom() {
            self.set_scroll_y(self.scroll_offset().y - (position.y - bounds.bottom()));
            inside.y = bounds.bottom() - px(1.);
        }
        if let Some(head) = self.offset_at(inside)
            && head != self.selection.head
        {
            self.selection.head = head;
            cx.notify();
        }
    }

    /// The button was released.
    pub(super) fn end_drag(&mut self) {
        self.selection.dragging = false;
    }

    /// Scrolls by `rows` rows (negative: up).
    pub(super) fn scroll_rows(&mut self, rows: f32, cx: &mut Context<Self>) {
        let y = self.scroll_offset().y - self.metrics.row_height * rows;
        self.set_scroll_y(y);
        cx.notify();
    }

    /// Scrolls to the first row, or with `last` to the last.
    pub(super) fn scroll_to_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let y = if last {
            -(self.metrics.row_height * self.row_count() as f32)
        } else {
            px(0.)
        };
        self.set_scroll_y(y);
        cx.notify();
    }

    /// The rows one screen holds.
    pub(super) fn page_rows(&self) -> f32 {
        match self.metrics.bounds {
            Some(bounds) if self.metrics.row_height > px(0.) => {
                (bounds.size.height / self.metrics.row_height)
                    .floor()
                    .max(1.)
                    - 1.
            }
            _ => 1.,
        }
    }
}

/// The word around `offset` (or just `offset` between words).
fn word_at(text: &str, offset: usize) -> (usize, usize) {
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| word_char(*ch))
        .last()
        .map_or(offset, |(at, _)| at);
    let end = text[offset..]
        .char_indices()
        .find(|(_, ch)| !word_char(*ch))
        .map_or(text.len(), |(at, _)| offset + at);
    (start, end)
}

/// The line around `offset`, with its newline.
fn line_at(text: &str, offset: usize) -> (usize, usize) {
    let start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at + 1);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_click_takes_the_word_and_a_triple_click_the_line() {
        let text = "metadata:\n  name: web-0\n  app.kubernetes.io/name: web\n";
        let at = text.find("web-0").unwrap() + 2;
        assert_eq!(&text[word_at(text, at).0..word_at(text, at).1], "web-0");
        let at = text.find("kubernetes").unwrap();
        let (s, e) = word_at(text, at);
        assert_eq!(&text[s..e], "app.kubernetes.io/name");
        let (s, e) = line_at(text, at);
        assert_eq!(&text[s..e], "  app.kubernetes.io/name: web\n");
        let (s, e) = word_at(text, text.find(": web-0").unwrap() + 1);
        assert_eq!(s, e, "between words nothing is selected");
    }

    #[test]
    fn a_selection_is_ordered_whichever_way_it_was_dragged() {
        let up = Selection {
            anchor: 9,
            head: 3,
            dragging: false,
        };
        assert_eq!(up.range(), Some(3..9));
        assert_eq!(Selection::default().range(), None);
    }
}
