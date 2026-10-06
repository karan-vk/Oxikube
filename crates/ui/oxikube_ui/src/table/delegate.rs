//! The data-source trait a table renders from.

use super::column::{SortDirection, TableColumn};
use super::text_cell::TextCell;
use crate::menu::PopupMenu;
use gpui::{App, IntoElement, ParentElement as _, Styled as _, Window, div};
use std::ops::Range;

/// What a [`Table`](super::Table) asks of its data.
///
/// Same shape as gpui-component's delegate (research §2) but with no library types in it:
/// callbacks get a plain `&mut App`, columns are [`TableColumn`]s, sort states are
/// [`SortDirection`]s.
///
/// Rows must have **uniform height** (the table is built on `uniform_list`): put variable-height
/// content in the detail drawer, not the row. Rendering is virtualised, so `render_td` is called
/// only for the rows on screen; keep it allocation-free where you can (return the element, do not
/// box it) and do the data work in `visible_rows_changed` or before updating the delegate.
pub trait TableDelegate: 'static {
    /// Number of columns.
    fn columns_count(&self, cx: &App) -> usize;

    /// Number of rows.
    fn rows_count(&self, cx: &App) -> usize;

    /// Describes column `col_ix`. Read when the table is created, [`refresh`]ed or the UI zoom
    /// changes, not per frame. Widths are design-time pixels: the table applies the zoom, so do not
    /// pass them through [`crate::u`].
    ///
    /// [`refresh`]: super::TableHandle::refresh
    fn column(&self, col_ix: usize, cx: &App) -> TableColumn;

    /// Renders the cell at (`row_ix`, `col_ix`).
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement;

    /// The cell at (`row_ix`, `col_ix`) when it is one line of text in one colour: the table
    /// draws it itself, which is cheaper than an element from [`render_td`](Self::render_td) (one
    /// element less, and the ellipsis only when the text does not fit its column; see
    /// [`TextCell`]). `None` (the default) draws the cell with `render_td`. Called for the visible
    /// cells every frame: hand out cached text.
    fn text_cell(
        &mut self,
        _row_ix: usize,
        _col_ix: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<TextCell> {
        None
    }

    /// Renders the header cell. Defaults to the column name.
    fn render_th(&mut self, col_ix: usize, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().child(self.column(col_ix, cx).name)
    }

    /// The user clicked a sortable header. Re-sort your data, then update the table.
    fn perform_sort(
        &mut self,
        _col_ix: usize,
        _direction: SortDirection,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    /// The user dragged column `from` to position `to` (only with
    /// [`TableOptions::movable_columns`](super::TableOptions)). Reorder your column list to match,
    /// or the next [`refresh`](super::TableHandle::refresh) will undo the move.
    fn move_column(&mut self, _from: usize, _to: usize, _window: &mut Window, _cx: &mut App) {}

    /// What to show when there are no rows.
    fn render_empty(&mut self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().size_full()
    }

    /// Whether to show the loading skeleton instead of rows.
    fn loading(&self, _cx: &App) -> bool {
        false
    }

    /// Whether more rows can be fetched when the user nears the bottom.
    fn has_more(&self, _cx: &App) -> bool {
        false
    }

    /// How many rows from the bottom [`load_more`](Self::load_more) fires.
    fn load_more_threshold(&self) -> usize {
        20
    }

    /// Fetch the next page. Called repeatedly near the bottom: guard it with your own flag.
    fn load_more(&mut self, _window: &mut Window, _cx: &mut App) {}

    /// The visible row range changed. Must be fast: it runs while scrolling.
    fn visible_rows_changed(
        &mut self,
        _visible_range: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    /// Whether row `row_ix` is part of the owner's selection, drawn with the selection
    /// background. Only consulted for tables whose owner selects
    /// ([`TableOptions::select_rows`](super::TableOptions) off); it runs for every visible row
    /// each frame, so keep it to a lookup.
    fn row_selected(&self, _row_ix: usize, _cx: &App) -> bool {
        false
    }

    /// The context menu of row `row_ix` (right click). Return `menu` unchanged for none.
    fn context_menu(
        &mut self,
        _row_ix: usize,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut App,
    ) -> PopupMenu {
        menu
    }

    /// Plain-text value of a cell, for copy and CSV export.
    fn cell_text(&self, _row_ix: usize, _col_ix: usize, _cx: &App) -> String {
        String::new()
    }
}
