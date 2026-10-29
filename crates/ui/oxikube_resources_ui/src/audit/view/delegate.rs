//! [`AuditDelegate`]: the table's data source over the [`AuditModel`].
//!
//! Cells are formatted when a visible row asks, so a frame that draws 40 rows formats 400 cells,
//! never the loaded 100 000. The table asks for the next page through `has_more` / `load_more`
//! when the visible rows near the end; the delegate only forwards that to the view.

use std::ops::Range;

use gpui::{App, IntoElement, ParentElement as _, Styled as _, WeakEntity, Window, div, px};
use oxikube_domain::audit::AuditOutcome;
use oxikube_ui::ActiveTokens as _;
use oxikube_ui::table::{TableColumn, TableDelegate, TextCell};

use super::AuditView;
use crate::audit::model::{AuditColumn, AuditModel, ClusterNames, cell_text};

/// The table's delegate. See the module docs.
pub struct AuditDelegate {
    /// The loaded rows.
    pub(crate) model: AuditModel,
    /// How the cluster cells are named.
    pub(crate) names: ClusterNames,
    /// An older page is being read.
    pub(crate) loading_more: bool,
    /// The first visible row, as last reported.
    first_visible: usize,
    view: WeakEntity<AuditView>,
}

impl AuditDelegate {
    pub(crate) fn new(view: WeakEntity<AuditView>) -> Self {
        Self {
            model: AuditModel::new(),
            names: ClusterNames::default(),
            loading_more: false,
            first_visible: usize::MAX,
            view,
        }
    }
}

impl TableDelegate for AuditDelegate {
    fn columns_count(&self, _: &App) -> usize {
        AuditColumn::ALL.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let column = AuditColumn::at(col_ix).unwrap_or(AuditColumn::Who);
        TableColumn::new(column.key(), column.title())
            .width(px(column.width()))
            .min_width(px(40.))
            .movable(false)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let text = self
            .text_cell(row_ix, col_ix, window, cx)
            .map(|cell| cell.text)
            .unwrap_or_default();
        div().child(text)
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<TextCell> {
        let column = AuditColumn::at(col_ix)?;
        let record = self.model.get(row_ix)?;
        let cell = TextCell::new(cell_text(record, column, &self.names));
        if column != AuditColumn::Outcome {
            return Some(cell);
        }
        let colors = cx.colors();
        Some(match record.outcome {
            AuditOutcome::Succeeded => cell,
            AuditOutcome::Failed => cell.color(colors.error),
            AuditOutcome::Denied => cell.color(colors.warning),
            AuditOutcome::Cancelled => cell.color(colors.text_muted),
        })
    }

    fn render_empty(&mut self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().size_full()
    }

    fn loading(&self, _: &App) -> bool {
        self.loading_more
    }

    fn has_more(&self, _: &App) -> bool {
        self.model.has_more()
    }

    fn load_more_threshold(&self) -> usize {
        50
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut App) {
        let view = self.view.clone();
        // Not inside the table's own update: the view reads the table.
        cx.defer(move |cx| {
            view.update(cx, |view, cx| view.load_more(cx)).ok();
        });
    }

    fn visible_rows_changed(&mut self, range: Range<usize>, _: &mut Window, cx: &mut App) {
        if range.start == self.first_visible {
            return;
        }
        self.first_visible = range.start;
        let (view, first) = (self.view.clone(), range.start);
        cx.defer(move |cx| {
            view.update(cx, |view, cx| view.on_scrolled(first, cx)).ok();
        });
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.model.selected() == Some(row_ix)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        match (self.model.get(row_ix), AuditColumn::at(col_ix)) {
            (Some(record), Some(column)) => cell_text(record, column, &self.names).to_string(),
            _ => String::new(),
        }
    }
}
