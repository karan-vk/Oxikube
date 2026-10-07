//! Scrolling: autoscroll that follows the newest line, pausing it when the user scrolls up (the
//! "N new lines" pill counts what arrives meanwhile), resuming at the bottom or from the pill,
//! and the anchor line that stays on screen while old lines are dropped or the wrap is toggled.
//!
//! Unwrapped rows all have [`LINE_HEIGHT`] and the `uniform_list`'s offset is in pixels; wrapped
//! rows are a `list` whose `ListState` is spliced with every delta, which keeps its scroll
//! position on the same row by itself.

use gpui::{Context, ListOffset, Pixels, ScrollStrategy, ScrollWheelEvent, Window, point, px};
use oxikube_runtime::notify_coalesced;
use oxikube_ui::u;

use super::LogView;
use super::window::{LineWindow, RowChange};

/// The height of an unwrapped row at 100 % zoom: one line of monospace text and its padding.
pub const LINE_HEIGHT: Pixels = px(20.);

/// Rows of context kept above a match that a search jump scrolls to.
const REVEAL_CONTEXT: usize = 3;

impl LogView {
    /// The height of an unwrapped row now (zoom applied).
    pub(crate) fn row_height(&self) -> Pixels {
        u(LINE_HEIGHT)
    }

    /// Replaces the rows (a new session): both renderers start over, at the end when following.
    pub(crate) fn reset_rows(&mut self, mut window: LineWindow) {
        if self.effective_levels().is_some() {
            window.set_visible(Some(std::collections::VecDeque::new()));
        }
        // The new session's seqs start at 0 again: nothing parsed or expanded carries over.
        self.records.borrow_mut().clear();
        self.expanded = None;
        self.saw_json = false;
        self.window = window;
        // A new stream numbers its lines from 0: the search starts over on it.
        self.reindex_empty();
        if self.options.wrap {
            self.list.reset(self.window.row_count());
        }
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(0.)));
        self.follow_tail();
    }

    /// Moves the renderers with the rows: the wrapped list is spliced; an unwrapped list that is
    /// not following keeps the same lines on screen when rows above them go.
    pub(crate) fn rows_changed(&mut self, change: RowChange) {
        if change.is_empty() {
            return;
        }
        if self.options.wrap {
            self.list
                .splice(0..change.front_removed, change.front_inserted);
            let start = change.front_inserted + change.kept;
            self.list
                .splice(start..start + change.tail_removed, change.tail_inserted);
        } else if !self.follow.is_on() && change.front_removed != change.front_inserted {
            let shift = change.front_removed as f32 - change.front_inserted as f32;
            let handle = &self.scroll.0.borrow().base_handle;
            let offset = handle.offset();
            let y = (offset.y + self.row_height() * shift).min(px(0.));
            handle.set_offset(point(offset.x, y));
        }
    }

    /// When following, scrolls to the newest row.
    pub(crate) fn follow_tail(&mut self) {
        if !self.follow.is_on() {
            return;
        }
        if self.options.wrap {
            self.list.scroll_to_end();
        } else {
            self.scroll.scroll_to_bottom();
        }
    }

    /// The row index at the top of the screen.
    pub fn top_row(&self) -> usize {
        if self.options.wrap {
            return self.list.logical_scroll_top().item_ix;
        }
        let offset = self.scroll.0.borrow().base_handle.offset();
        let rows = (-offset.y / self.row_height()).floor();
        if rows.is_finite() && rows > 0. {
            rows as usize
        } else {
            0
        }
    }

    /// The seq of the line at the top of the screen (the anchor line).
    pub fn top_seq(&self) -> Option<u64> {
        self.window.seq_near(self.top_row())
    }

    /// Puts row `index` at the top of the screen.
    pub(crate) fn scroll_to_row(&mut self, index: usize) {
        if self.options.wrap {
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        } else {
            self.scroll
                .scroll_to_item_strict(index, ScrollStrategy::Top);
        }
    }

    /// Puts the line `seq` at the top of the screen (when it is still retained).
    pub(crate) fn scroll_to_seq(&mut self, seq: u64) {
        if let Some(index) = self.window.index_of(seq) {
            self.scroll_to_row(index);
        }
    }

    /// Puts the line `seq` a few rows below the top of the screen (a search match, with some of
    /// the lines before it for context). Does nothing when it is not a row.
    pub(crate) fn reveal_seq(&mut self, seq: u64) {
        if let Some(index) = self.window.index_of(seq) {
            self.scroll_to_row(index.saturating_sub(REVEAL_CONTEXT));
        }
    }

    /// The rows were replaced wholesale (a search started, stopped or changed mode): the wrapped
    /// list starts over, and the screen shows the newest line when following, else the line that
    /// was at the top (or the nearest one still shown).
    pub(crate) fn rows_rebuilt(&mut self, anchor: Option<u64>) {
        if self.options.wrap {
            self.list.reset(self.window.row_count());
        }
        if self.follow.is_on() {
            self.follow_tail();
        } else if let Some(row) = anchor.and_then(|seq| self.window.row_near_seq(seq)) {
            self.scroll_to_row(row);
        }
    }

    /// Switches the renderer for the wrap option, keeping the anchor line at the top (or the
    /// tail, when following).
    pub(crate) fn switch_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if self.options.wrap == wrap {
            return;
        }
        let anchor = self.top_seq();
        self.options.wrap = wrap;
        if wrap {
            self.list.reset(self.window.row_count());
        }
        if self.follow.is_on() {
            self.follow_tail();
        } else if let Some(seq) = anchor {
            self.scroll_to_seq(seq);
        }
        cx.notify();
    }

    /// Pauses autoscroll; lines from now on count for the pill.
    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        if self.follow.is_on() {
            self.follow.pause(self.window.next_seq());
            if self.options.wrap {
                // Stop the list where it is (`scroll_to_end` left it pinned to the end).
                let top = self.list.logical_scroll_top();
                self.list.scroll_to(top);
            }
            cx.notify();
        }
    }

    /// Resumes autoscroll and jumps to the newest line (the pill, `s`).
    pub(crate) fn resume(&mut self, cx: &mut Context<Self>) {
        self.follow.resume();
        self.follow_tail();
        cx.notify();
    }

    /// A scroll wheel over the rows: up pauses autoscroll; down to the bottom resumes it.
    pub(crate) fn on_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dy = event.delta.pixel_delta(self.row_height()).y;
        if dy > px(0.) {
            self.pause(cx);
        } else if dy < px(0.) && !self.follow.is_on() {
            // The list moves in its own handler; look once it has.
            cx.on_next_frame(window, |view, _, cx| {
                if view.at_end() {
                    view.resume(cx);
                }
            });
        }
        notify_coalesced(cx);
    }

    /// Whether the newest row is on screen (as of the last frame).
    pub fn at_end(&self) -> bool {
        let at_end = if self.options.wrap {
            self.list.is_scrolled_to_end()
        } else {
            self.scroll.is_scrolled_to_end()
        };
        // Not scrollable: everything is on screen.
        at_end.unwrap_or(true)
    }
}
