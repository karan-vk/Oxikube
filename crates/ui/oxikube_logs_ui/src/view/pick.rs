//! Which lines an action takes: the ones on screen, the whole buffer, the filtered ones, and how
//! they are written (as drawn).

use std::ops::Range;

use gpui::Context;
use oxikube_app::logs::export::{ExportFormat, ExportSpec, LineFilter};

use super::LogView;
use super::window::Row;

/// The most rows asked of a wrapped list's layout when looking for what is on screen.
const MAX_WRAPPED_ROWS: usize = 1_000;

impl LogView {
    /// The rows on screen now, as of the last frame.
    pub(crate) fn viewport_rows(&self) -> Range<usize> {
        let top = self.top_row();
        let count = if self.options.wrap {
            self.wrapped_rows_on_screen()
        } else {
            let height = self.scroll.0.borrow().last_item_size.map(|s| s.item.height);
            height.map_or(0, |height| {
                let rows = (height / self.row_height()).ceil();
                if rows.is_finite() && rows > 0. {
                    rows as usize
                } else {
                    0
                }
            })
        };
        top..(top + count).min(self.window.row_count())
    }

    fn wrapped_rows_on_screen(&self) -> usize {
        let viewport = self.list.viewport_bounds();
        let top = self.list.logical_scroll_top().item_ix;
        (top..self.window.row_count().min(top + MAX_WRAPPED_ROWS))
            .map(|ix| self.list.bounds_for_item(ix))
            .take_while(|bounds| bounds.is_some_and(|b| b.top() < viewport.bottom()))
            .count()
    }

    /// The seqs of the lines on screen (the marker and state rows are not lines); `None` when no
    /// line is on screen.
    pub fn viewport_seqs(&self) -> Option<Range<u64>> {
        let mut seqs = self
            .viewport_rows()
            .filter_map(|ix| match self.window.row(ix) {
                Some(Row::Line(seq)) => Some(seq),
                _ => None,
            });
        let first = seqs.next()?;
        let last = seqs.next_back().unwrap_or(first);
        Some(first..last + 1)
    }

    /// How saved and copied lines are written by default: as the view draws them (the timestamp
    /// when it is shown), without a pod prefix (a single pod's view has one pod).
    pub fn export_format(&self) -> ExportFormat {
        ExportFormat {
            timestamps: self.options.timestamps,
            pod_prefix: false,
        }
    }

    /// The filter a copy or a save applies: only the lines it accepts are taken ("what you see is
    /// what you export"). The search and filter bar installs its matcher here; without one every
    /// line passes.
    pub fn line_filter(&self) -> Option<&LineFilter> {
        self.filter.as_ref()
    }

    /// Installs (or removes) the filter that copies and saves apply.
    pub fn set_line_filter(&mut self, filter: Option<LineFilter>, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }

    /// What to take for `seqs` in `format`: the lines of that range that pass the filter.
    pub(crate) fn spec_for(&self, seqs: Range<u64>, format: ExportFormat) -> ExportSpec {
        ExportSpec::new(seqs, format).with_filter(self.filter.clone())
    }
}
