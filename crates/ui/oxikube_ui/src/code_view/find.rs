//! Showing the matches of a find (E11-S06): a background on every match, a stronger one on the
//! current match, and scrolling to it.
//!
//! The view does not search: the owner finds the matches (off the UI thread, over the same
//! `Arc<str>` it gave [`CodeView::set_text`]) and hands over their byte ranges. Ranges are drawn
//! only while the text they were found in is the text on screen, so a newer text never gets old
//! offsets; the owner finds again for the new text. A frame looks up the ranges of the rows it
//! draws by binary search, so thousands of matches cost nothing per frame.

use std::ops::Range;
use std::sync::Arc;

use gpui::Context;

use super::CodeView;
use super::view::Matches;

impl CodeView {
    /// Colours `ranges` (sorted, not overlapping) of `text`, and `current` (an index into
    /// `ranges`) apart from the others.
    pub fn set_matches(
        &mut self,
        text: Arc<str>,
        ranges: Arc<[Range<usize>]>,
        current: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.matches = Some(Matches {
            text,
            ranges,
            current,
        });
        cx.notify();
    }

    /// Stops colouring matches.
    pub fn clear_matches(&mut self, cx: &mut Context<Self>) {
        if self.matches.take().is_some() {
            cx.notify();
        }
    }

    /// Scrolls so the row holding `byte` of the text on screen is in view, a third of the way
    /// down when it was not (so the lines before it show too). Does nothing without a text or
    /// before the first frame measured the view.
    pub fn scroll_to_byte(&mut self, byte: usize, cx: &mut Context<Self>) {
        let (Some(shown), Some(bounds)) = (self.shown.as_ref(), self.metrics.bounds) else {
            return;
        };
        let row_height = self.metrics.row_height;
        if row_height <= gpui::px(0.) {
            return;
        }
        let row = shown.rows.row_of_byte(byte) as f32;
        let top = -self.scroll_offset().y / row_height;
        let visible = (bounds.size.height / row_height).floor().max(1.);
        // Already fully in view: leave the scroll alone.
        if row >= top.ceil() && row + 1. <= top.floor() + visible {
            return;
        }
        let target = (row - (visible / 3.).floor()).max(0.);
        self.set_scroll_y(-(row_height * target));
        cx.notify();
    }

    /// The byte where the first row on screen starts: where a find starts looking when no match
    /// is current. 0 before there is a text.
    pub fn top_byte(&self) -> usize {
        self.shown
            .as_ref()
            .and_then(|shown| shown.rows.row(self.top_row()))
            .map_or(0, |row| row.start)
    }

    /// The first row of the screen (for tests).
    pub fn top_visible_row(&self) -> usize {
        self.top_row()
    }

    /// The rows the viewport holds (for tests); 0 before the first frame.
    pub fn visible_rows(&self) -> usize {
        match self.metrics.bounds {
            Some(bounds) if self.metrics.row_height > gpui::px(0.) => {
                (bounds.size.height / self.metrics.row_height).floor() as usize
            }
            _ => 0,
        }
    }
}
