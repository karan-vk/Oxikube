//! [`EventsDelegate`]: the table's data source over the [`EventsModel`].
//!
//! Every cell is a [`TextCell`] handed out from the row's cached text (the relative ones cached
//! per tick), so drawing the visible rows formats nothing on a frame where nothing changed. The
//! selection is the model's (it follows the event, not the row number); the table only reports
//! clicks. Scrolling away from the top tells the view, which pauses the live tail so rows do not
//! move under the pointer.

use std::ops::Range;

use gpui::{App, IntoElement, ParentElement as _, Styled as _, WeakEntity, Window, div, px};
use jiff::Timestamp;
use oxikube_ui::ActiveTokens as _;
use oxikube_ui::table::{SortDirection, TableColumn, TableDelegate, TextCell};

use super::EventsView;
use crate::model::{EventColumn, EventSort, EventsModel};

/// The table's delegate. See the [module docs](self).
pub struct EventsDelegate {
    /// The rows.
    pub(crate) model: EventsModel,
    /// The time the relative cells are worked out against.
    pub(crate) now: Timestamp,
    /// Names `now`: the rows cache their relative cells per epoch.
    pub(crate) epoch: u64,
    /// The first visible row, as last reported.
    pub(crate) first_visible: usize,
    view: WeakEntity<EventsView>,
}

impl EventsDelegate {
    pub(crate) fn new(view: WeakEntity<EventsView>, now: Timestamp) -> Self {
        Self {
            model: EventsModel::new(),
            now,
            epoch: 0,
            first_visible: 0,
            view,
        }
    }

    /// Moves the clock of the relative cells to `now`.
    pub(crate) fn tick(&mut self, now: Timestamp) {
        self.now = now;
        self.epoch += 1;
    }
}

fn direction(sort: EventSort, column: EventColumn) -> SortDirection {
    match (sort.column == column, sort.descending) {
        (false, _) => SortDirection::Unsorted,
        (true, true) => SortDirection::Descending,
        (true, false) => SortDirection::Ascending,
    }
}

impl TableDelegate for EventsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        EventColumn::ALL.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let column = EventColumn::at(col_ix).unwrap_or(EventColumn::Message);
        let mut out = TableColumn::new(column.key(), column.title())
            .width(px(column.width()))
            .min_width(px(40.))
            .movable(false)
            .sorted(direction(self.model.sort(), column));
        if column == EventColumn::Count {
            out = out.right();
        }
        out
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
        let column = EventColumn::at(col_ix)?;
        let (now, epoch) = (self.now, self.epoch);
        let row = self.model.rows_mut().get_mut(row_ix)?;
        let cell = TextCell::new(row.cell(column, now, epoch));
        Some(if column == EventColumn::Type && row.is_warning() {
            cell.color(cx.colors().warning)
        } else {
            cell
        })
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        direction: SortDirection,
        _: &mut Window,
        _: &mut App,
    ) {
        let sort = match (EventColumn::at(col_ix), direction) {
            (Some(column), SortDirection::Ascending) => EventSort {
                column,
                descending: false,
            },
            (Some(column), SortDirection::Descending) => EventSort {
                column,
                descending: true,
            },
            // Back to the default: last seen, newest first.
            _ => EventSort::default(),
        };
        self.model.set_sort(sort);
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.text_muted)
            .child("No events")
    }

    fn visible_rows_changed(&mut self, range: Range<usize>, _: &mut Window, cx: &mut App) {
        if range.start == self.first_visible {
            return;
        }
        self.first_visible = range.start;
        let view = self.view.clone();
        let first = range.start;
        // Not inside the table's own update: the view reads the table.
        cx.defer(move |cx| {
            view.update(cx, |view, cx| view.on_scrolled(first, cx)).ok();
        });
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.model.is_selected(row_ix)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        match (self.model.rows().get(row_ix), EventColumn::at(col_ix)) {
            (Some(row), Some(column)) => row.fixed(column).to_owned(),
            _ => String::new(),
        }
    }
}
