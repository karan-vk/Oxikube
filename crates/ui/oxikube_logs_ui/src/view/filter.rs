//! The level chips (E08-S05): which levels the rows show, and keeping the window's row index in
//! step with them.
//!
//! The filter is one predicate over a buffered line ([`LevelFilter::admits`]) and composes with
//! the search (E08-S03): the rows are the lines that pass the chips and, in the search's filter
//! mode, also match. It applies in JSON mode only: off, every line is raw text and nothing is
//! hidden (the chips are kept for when it is on again). While it hides something the
//! [`LineWindow`](super::LineWindow) keeps the seqs of the lines that are rows; a delta is
//! filtered as it is applied (its candidate lines, not the buffer), and a changed filter, search
//! or mode is one pass over the window's candidate lines ([`LogView::relevel`]).

use gpui::Context;
use oxikube_app::logs::{LevelFilter, LogSession};
use oxikube_domain::log::LevelChip;

use super::LogView;

impl LogView {
    /// The filter in effect: `Some` while JSON mode is on and a chip is off, else `None` (every
    /// line is a row).
    pub(crate) fn effective_levels(&self) -> Option<LevelFilter> {
        (self.options.json && !self.levels.is_all()).then_some(self.levels)
    }

    /// Whether the JSON controls (the toggle and the chips) are shown: once the session delivered
    /// a structured line, and always while JSON mode is off or a chip is (so they can be undone).
    pub(crate) fn shows_json_controls(&self) -> bool {
        self.saw_json || !self.options.json || !self.levels.is_all()
    }

    /// The level chips as set (what JSON mode would apply).
    pub fn levels(&self) -> LevelFilter {
        self.levels
    }

    /// Turns the level `chip` on or off (`logs::ToggleLevel`).
    pub fn toggle_level(&mut self, chip: LevelChip, cx: &mut Context<Self>) {
        self.levels.toggle(chip);
        self.refilter(cx);
    }

    /// JSON mode on or off (`logs::ToggleJsonMode`).
    pub fn toggle_json_mode(&mut self, cx: &mut Context<Self>) {
        self.set_json_mode(!self.options.json, cx);
    }

    /// Sets JSON mode (what the key and the `logs.json_auto_detect` setting both do).
    pub(crate) fn set_json_mode(&mut self, json: bool, cx: &mut Context<Self>) {
        if self.options.json == json {
            return;
        }
        self.options.json = json;
        if self.options.wrap {
            // Columns and raw text wrap to different heights.
            self.list.remeasure();
        }
        self.refilter(cx);
    }

    /// Rebuilds which lines are rows after the filter changed, keeping the line at the top of the
    /// screen (or the first one after it, when it was hidden), or the tail when following.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let anchor = self.top_seq();
        self.relevel();
        // The pane belongs to a shown structured line: raw text mode has none, and a chip may
        // have hidden the line.
        let pane_line_shown = self
            .expanded
            .as_ref()
            .is_none_or(|e| self.options.json && self.window.index_of(e.seq).is_some());
        if !pane_line_shown {
            self.expanded = None;
        }
        if self.options.wrap {
            self.list.reset(self.window.row_count());
        }
        if self.follow.is_on() {
            self.follow_tail();
        } else if let Some(index) = anchor.and_then(|seq| self.window.row_near_seq(seq)) {
            self.scroll_to_row(index);
        }
        cx.notify();
    }

    /// Rebuilds the window's list of rows that pass the level chips from its candidate lines (the
    /// search's matches while it narrows the rows, every retained line otherwise). Called after
    /// the chips, JSON mode or the search's index or mode changed; the caller rebuilds the
    /// renderers.
    pub(crate) fn relevel(&mut self) {
        let visible = self.effective_levels().map(|filter| {
            admitted(
                self.session.as_ref(),
                filter,
                self.window.candidate_seqs(),
            )
        });
        self.window.set_visible(visible);
    }
}

/// The seqs in `seqs` (ascending) that `filter` admits, in order (none without a session; a seq
/// the buffer no longer holds is not admitted).
pub(super) fn admitted<C: FromIterator<u64> + Default>(
    session: Option<&LogSession>,
    filter: LevelFilter,
    seqs: impl Iterator<Item = u64>,
) -> C {
    session.map_or_else(C::default, |session| {
        session.read(|buffer, _| {
            seqs.filter(|seq| {
                buffer
                    .get_seq(*seq)
                    .is_some_and(|entry| filter.admits(entry))
            })
            .collect()
        })
    })
}
