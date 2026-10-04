//! The retained table state: [`TableHandle`] and [`TableEvent`].

use super::adapter::Adapter;
use super::delegate::TableDelegate;
use super::widths::ColumnWidths;
use crate::size::Unscaled;
use gpui::{App, AppContext as _, Entity, Subscription, Window};
use gpui_component::table::{TableEvent as LibEvent, TableState as LibState};
use std::rc::Rc;

/// Something happened in a table that its owner may care about.
#[derive(Clone, Debug, PartialEq)]
pub enum TableEvent {
    /// A row was selected (click or keyboard).
    SelectRow(usize),
    /// A row was double-clicked (or Enter): open it.
    ActivateRow(usize),
    /// A row (or the empty area, `None`) was right-clicked: show a context menu.
    RightClickedRow(Option<usize>),
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
    fn from_library(event: &LibEvent, widths: &ColumnWidths) -> Option<Self> {
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

/// Behaviour switches fixed when the table is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableOptions {
    /// Clicking a header sorts (columns must also be [`sortable`](super::TableColumn::sortable)).
    pub sortable: bool,
    /// Columns can be resized by dragging.
    pub resizable_columns: bool,
    /// Columns can be reordered by dragging.
    pub movable_columns: bool,
    /// Arrow-key selection wraps at the ends.
    pub loop_selection: bool,
}

impl Default for TableOptions {
    fn default() -> Self {
        Self {
            sortable: true,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
        }
    }
}

/// A table's retained state: the delegate, scroll position and selection. Cheap to clone (it is a
/// handle); render it with [`Table::new`](super::Table::new).
pub struct TableHandle<D: TableDelegate> {
    state: Entity<LibState<Adapter<D>>>,
    widths: Rc<ColumnWidths>,
}

impl<D: TableDelegate> Clone for TableHandle<D> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            widths: self.widths.clone(),
        }
    }
}

impl<D: TableDelegate> TableHandle<D> {
    /// Creates a table over `delegate` with default [`TableOptions`].
    pub fn new(delegate: D, window: &mut Window, cx: &mut App) -> Self {
        Self::with_options(delegate, TableOptions::default(), window, cx)
    }

    /// Creates a table over `delegate`.
    pub fn with_options(
        delegate: D,
        options: TableOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let widths = Rc::new(ColumnWidths::new());
        let adapter_widths = widths.clone();
        let state = cx.new(|cx| {
            LibState::new(Adapter::new(delegate, adapter_widths), window, cx)
                .sortable(options.sortable)
                .col_resizable(options.resizable_columns)
                .col_movable(options.movable_columns)
                .loop_selection(options.loop_selection)
        });
        // Remember what the user resized (unscaled), whether or not anyone listens for events, so
        // a zoom change keeps those widths. The subscription ends with the table's state.
        let recorder = widths.clone();
        cx.subscribe(&state, move |_, event: &LibEvent, _| {
            if let LibEvent::ColumnWidthsChanged(resized) = event {
                recorder.record_resize(resized);
            }
        })
        .detach();
        Self { state, widths }
    }

    pub(super) fn state(&self) -> &Entity<LibState<Adapter<D>>> {
        &self.state
    }

    /// Reads the delegate.
    pub fn read<R>(&self, cx: &App, f: impl FnOnce(&D) -> R) -> R {
        f(&self.state.read(cx).delegate().delegate)
    }

    /// Mutates the delegate (new rows, new sort order) and re-renders the table. Keeps column
    /// widths and scroll position; call [`refresh`](Self::refresh) too if the columns changed.
    pub fn update<R>(&self, cx: &mut App, f: impl FnOnce(&mut D) -> R) -> R {
        self.state.update(cx, |state, cx| {
            let result = f(&mut state.delegate_mut().delegate);
            cx.notify();
            result
        })
    }

    /// Re-reads the column definitions from the delegate. Resets user-resized widths (use it when
    /// the columns themselves changed; a UI zoom change is handled by the table, keeping them).
    pub fn refresh(&self, cx: &mut App) {
        self.widths.clear_user_widths();
        self.reread_columns(cx);
        self.state.update(cx, |_, cx| cx.notify());
    }

    /// Re-reads the columns (at the current zoom) without forgetting user-resized widths.
    fn reread_columns(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| state.refresh(cx));
        self.widths.mark_applied();
    }

    /// Called by the element each frame: the library caches widths, so a zoom change since the
    /// last read means they are stale.
    pub(super) fn rescale_if_stale(&self, cx: &mut App) {
        if self.widths.is_stale() {
            self.reread_columns(cx);
        }
    }

    /// The selected row, if any.
    pub fn selected_row(&self, cx: &App) -> Option<usize> {
        self.state.read(cx).selected_row()
    }

    /// Selects `row_ix`.
    pub fn select_row(&self, row_ix: usize, cx: &mut App) {
        self.state
            .update(cx, |state, cx| state.set_selected_row(row_ix, cx));
    }

    /// Clears the selection.
    pub fn clear_selection(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| state.clear_selection(cx));
    }

    /// Scrolls `row_ix` to the top of the viewport.
    pub fn scroll_to_row(&self, row_ix: usize, cx: &mut App) {
        self.state
            .update(cx, |state, cx| state.scroll_to_row(row_ix, cx));
    }

    /// The row range currently on screen (empty before the first layout).
    pub fn visible_rows(&self, cx: &App) -> std::ops::Range<usize> {
        self.state.read(cx).visible_range().rows().clone()
    }

    /// Calls `handler` for every [`TableEvent`]. Keep the returned subscription alive.
    pub fn on_event(
        &self,
        cx: &mut App,
        mut handler: impl FnMut(&TableEvent, &mut App) + 'static,
    ) -> Subscription {
        let widths = self.widths.clone();
        cx.subscribe(&self.state, move |_, event: &LibEvent, cx| {
            if let Some(event) = TableEvent::from_library(event, &widths) {
                handler(&event, cx);
            }
        })
    }
}
