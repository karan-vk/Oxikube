//! Selecting lines and marking them (E08-S06): the models, and the pointer handling.
//!
//! Both are anchored by **seq**, never by row index: scrolling, the autoscroll, the ring buffer
//! dropping lines above, or a wrap toggle never move a selection or a mark off its line (Freelens
//! #1170 lost the selection when the list scrolled). A click selects a line (and makes it the
//! focused one, the line `m` marks), shift-click or dragging across rows extends the selection
//! from where it started. Plain Rust here; the view's methods are below.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use gpui::Context;

use super::LogView;

/// The selected lines: an anchor (where the click or drag began) and a cursor (where it is now,
/// the focused line). Both are seqs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Selection {
    anchor: Option<u64>,
    cursor: Option<u64>,
}

impl Selection {
    /// Selects just `seq` (a click).
    pub fn select(&mut self, seq: u64) {
        self.anchor = Some(seq);
        self.cursor = Some(seq);
    }

    /// Extends the selection to `seq` (shift-click, dragging); with nothing selected it selects
    /// `seq`.
    pub fn extend_to(&mut self, seq: u64) {
        if self.anchor.is_none() {
            self.anchor = Some(seq);
        }
        self.cursor = Some(seq);
    }

    /// Selects nothing.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The focused line: where the selection was last clicked or dragged to.
    pub fn cursor(&self) -> Option<u64> {
        self.cursor
    }

    /// The selected seqs, first to last, whichever way it was dragged.
    pub fn range(&self) -> Option<RangeInclusive<u64>> {
        let (anchor, cursor) = (self.anchor?, self.cursor?);
        Some(anchor.min(cursor)..=anchor.max(cursor))
    }

    /// Whether `seq` is selected.
    pub fn contains(&self, seq: u64) -> bool {
        self.range().is_some_and(|range| range.contains(&seq))
    }

    /// The buffer no longer holds seqs below `first_seq`: a selection that was entirely in what
    /// went is dropped; one that reaches into what stays keeps its ends (what a copy reads is
    /// what is still there).
    pub fn forget_below(&mut self, first_seq: u64) {
        if self.range().is_some_and(|range| *range.end() < first_seq) {
            self.clear();
        }
    }
}

/// The marked lines (k9s `m`): a set of seqs, shown as a gutter bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks(BTreeSet<u64>);

impl Marks {
    /// Marks `seq`, or unmarks it when it is marked. Returns whether it is marked now.
    pub fn toggle(&mut self, seq: u64) -> bool {
        if self.0.remove(&seq) {
            false
        } else {
            self.0.insert(seq);
            true
        }
    }

    /// Whether `seq` is marked.
    pub fn contains(&self, seq: u64) -> bool {
        self.0.contains(&seq)
    }

    /// How many lines are marked.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing is marked.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The marked seqs, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.0.iter().copied()
    }

    /// Forgets the marks on lines the buffer no longer holds (`seq < first_seq`): a mark lives
    /// only as long as its line.
    pub fn retain_from(&mut self, first_seq: u64) {
        if self.0.first().is_some_and(|&first| first < first_seq) {
            self.0 = self.0.split_off(&first_seq);
        }
    }

    /// Removes every mark.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl LogView {
    /// A click on the line `seq`: selects it and makes it the focused line; with `extend` (shift)
    /// the selection grows from where it started instead.
    pub fn click_line(&mut self, seq: u64, extend: bool, cx: &mut Context<Self>) {
        if extend {
            self.selection.extend_to(seq);
        } else {
            self.selection.select(seq);
        }
        cx.notify();
    }

    /// The pointer dragged over the line `seq` with the button down: extends the selection to it.
    pub fn drag_to_line(&mut self, seq: u64, cx: &mut Context<Self>) {
        if self.selection.cursor() != Some(seq) {
            self.selection.extend_to(seq);
            cx.notify();
        }
    }

    /// Selects nothing.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        cx.notify();
    }

    /// The selected seqs (first to last); `None` when nothing is selected.
    pub fn selection(&self) -> Option<RangeInclusive<u64>> {
        self.selection.range()
    }

    /// The marked seqs, oldest first.
    pub fn marked(&self) -> Vec<u64> {
        self.marks.iter().collect()
    }

    /// Whether the line `seq` is marked.
    pub fn is_marked(&self, seq: u64) -> bool {
        self.marks.contains(seq)
    }

    /// The focused line: the one last clicked, else the line at the top of the screen.
    pub fn focused_seq(&self) -> Option<u64> {
        self.selection.cursor().or_else(|| self.top_seq())
    }

    /// Marks the focused line, or unmarks it (`m`, `logs::Mark`).
    pub fn toggle_mark(&mut self, cx: &mut Context<Self>) {
        if let Some(seq) = self.focused_seq() {
            self.marks.toggle(seq);
            cx.notify();
        }
    }

    /// The buffer moved on: marks and a selection that are entirely on lines it dropped go too.
    pub(crate) fn forget_dropped(&mut self) {
        let first = self.window.first_seq();
        self.marks.retain_from(first);
        self.selection.forget_below(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_anchored_and_grows_either_way() {
        let mut selection = Selection::default();
        assert_eq!(selection.range(), None);
        selection.select(10);
        assert_eq!(selection.range(), Some(10..=10));
        selection.extend_to(14);
        assert_eq!(selection.range(), Some(10..=14));
        selection.extend_to(7);
        assert_eq!(
            selection.range(),
            Some(7..=10),
            "dragged back past the anchor"
        );
        assert_eq!(selection.cursor(), Some(7));
        assert!(selection.contains(8) && !selection.contains(11));
        selection.clear();
        assert_eq!(selection.range(), None);
        selection.extend_to(3);
        assert_eq!(
            selection.range(),
            Some(3..=3),
            "extending nothing selects the line"
        );
    }

    #[test]
    fn dropped_lines_take_a_selection_only_when_all_of_it_went() {
        let mut selection = Selection::default();
        selection.select(5);
        selection.extend_to(20);
        selection.forget_below(10);
        assert_eq!(
            selection.range(),
            Some(5..=20),
            "it reaches into what stays"
        );
        selection.forget_below(21);
        assert_eq!(selection.range(), None);
    }

    #[test]
    fn marks_toggle_and_live_only_while_their_line_does() {
        let mut marks = Marks::default();
        assert!(marks.toggle(3));
        assert!(marks.toggle(8));
        assert!(!marks.toggle(3), "a second toggle unmarks");
        assert!(marks.toggle(12));
        assert_eq!(marks.iter().collect::<Vec<_>>(), [8, 12]);
        marks.retain_from(9);
        assert_eq!(marks.iter().collect::<Vec<_>>(), [12]);
        assert_eq!(marks.len(), 1);
        marks.clear();
        assert!(marks.is_empty());
    }
}
