//! What a table tells its owner: [`TableEvent`] and [`RowClick`].

use super::column::SortDirection;
use super::widths::ColumnWidths;
use crate::size::Unscaled;
use gpui::{ClickEvent, EventEmitter};
use gpui_component::table::TableEvent as LibEvent;

/// A click on a row, with what an owner that keeps its own selection needs to tell a plain click
/// from a range or a toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowClick {
    /// The row clicked.
    pub row: usize,
    /// Shift was held: extend the selection to this row.
    pub extend: bool,
    /// The platform's secondary modifier was held (cmd on macOS, ctrl elsewhere): toggle this
    /// row in the selection.
    pub toggle: bool,
    /// 1 for a click, 2 for a double click, ...
    pub count: usize,
}

impl RowClick {
    pub(super) fn from_click(row: usize, event: &ClickEvent) -> Self {
        let modifiers = event.modifiers();
        Self {
            row,
            extend: modifiers.shift,
            toggle: modifiers.secondary(),
            count: event.click_count(),
        }
    }
}

/// Something happened in a table that its owner may care about.
#[derive(Clone, Debug, PartialEq)]
pub enum TableEvent {
    /// A row was selected (click or keyboard) by a table that selects rows itself.
    SelectRow(usize),
    /// A row was double-clicked (or Enter) in a table that selects rows itself: open it.
    ActivateRow(usize),
    /// A row was clicked, with its modifiers. Sent by every table, before any
    /// [`SelectRow`](Self::SelectRow) the click causes; an owner that keeps its own selection
    /// ([`TableOptions::select_rows`](super::TableOptions) off) selects from this.
    RowClicked(RowClick),
    /// A row (or the empty area, `None`) was right-clicked: show a context menu.
    RightClickedRow(Option<usize>),
    /// The user clicked a sortable header: the table asks for column `column` in `direction`
    /// ([`SortDirection::Unsorted`] after the third click). The delegate's
    /// [`perform_sort`](super::TableDelegate::perform_sort) has run.
    SortChanged {
        /// The column, by index.
        column: usize,
        /// The direction asked for.
        direction: SortDirection,
    },
    /// The user resized columns; widths in column order, unscaled (persist them as they are; the
    /// table applies the zoom when it reads them back).
    ColumnsResized(Vec<Unscaled>),
    /// The user dragged column `from` to position `to`.
    ColumnMoved {
        /// Index the column had.
        from: usize,
        /// Index it now has.
        to: usize,
    },
    /// The selection was cleared.
    SelectionCleared,
}

impl TableEvent {
    pub(super) fn from_library(event: &LibEvent, widths: &ColumnWidths) -> Option<Self> {
        Some(match event {
            LibEvent::SelectRow(ix) => TableEvent::SelectRow(*ix),
            LibEvent::DoubleClickedRow(ix) => TableEvent::ActivateRow(*ix),
            LibEvent::RightClickedRow(ix) => TableEvent::RightClickedRow(*ix),
            LibEvent::ColumnWidthsChanged(resized) => {
                TableEvent::ColumnsResized(widths.unscale(resized))
            }
            LibEvent::MoveColumn(from, to) => TableEvent::ColumnMoved {
                from: *from,
                to: *to,
            },
            LibEvent::ClearSelection => TableEvent::SelectionCleared,
            // Column and cell selection are not used: tables select whole rows.
            LibEvent::SelectColumn(_)
            | LibEvent::SelectCell(..)
            | LibEvent::DoubleClickedCell(..)
            | LibEvent::RightClickedCell(..) => return None,
        })
    }
}

/// Emits the events the library has none for (row clicks with modifiers, sort changes). One per
/// table, held by its handle and its adapter.
pub(super) struct TableEvents;

impl EventEmitter<TableEvent> for TableEvents {}
