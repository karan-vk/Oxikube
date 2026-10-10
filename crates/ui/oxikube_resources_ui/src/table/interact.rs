//! What the user does to a table: clicks (plain, shift-range, cmd/ctrl-toggle, double), keys
//! (the [`actions`](super::actions)), header sorts, column drags and resizes, the column picker,
//! and the commands those dispatch.
//!
//! Selection and cursor moves are view state and redraw at once (`cx.notify()`, input is not
//! coalesced). Opening, copying a name and selecting all are commands
//! (`resource::Open`, `resource::CopyName`, `resource::SelectAll`) sent through the
//! dispatcher; the bus hands them back to [`ResourceViews`](crate::ResourceViews), which calls
//! [`ResourceTable::open_detail`], [`ResourceTable::select_all`] or writes the clipboard.

use gpui::{Context, Window};
use oxikube_app::ColumnId;
use oxikube_app::store::ObjectKey;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::table::{RowClick, TableEvent};

use super::actions::{
    ClearSelection, CopyName, ExtendNext, ExtendPrevious, OpenSelected, SelectAll, SelectFirst,
    SelectHalfPageDown, SelectHalfPageUp, SelectLast, SelectNext, SelectPageDown, SelectPageUp,
    SelectPrevious,
};
use super::selection::ClickMode;
use super::view::{ResourceTable, ResourceTableEvent};

impl ResourceTable {
    pub(crate) fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        match event {
            TableEvent::RowClicked(click) => self.on_row_click(*click, cx),
            TableEvent::RightClickedRow(Some(row)) => {
                let row = *row;
                self.table
                    .update(cx, |d| d.selection.context_click(&d.rows, row));
                self.selection_changed(cx);
            }
            TableEvent::SortChanged { .. } => {
                // The delegate already took the new sort into its layout.
                self.apply_sort(cx);
                self.save_prefs(cx);
            }
            TableEvent::ColumnsResized(widths) => {
                let widths: Vec<f32> = widths.iter().map(|w| w.0).collect();
                if self
                    .table
                    .update_quiet(cx, |d| d.layout.set_widths(&widths))
                {
                    self.save_prefs(cx);
                }
            }
            TableEvent::ColumnMoved { .. } => self.save_prefs(cx),
            TableEvent::RightClickedRow(None)
            | TableEvent::SelectRow(_)
            | TableEvent::ActivateRow(_)
            | TableEvent::SelectionCleared => {}
        }
    }

    fn on_row_click(&mut self, click: RowClick, cx: &mut Context<Self>) {
        let mode = if click.extend {
            ClickMode::Extend
        } else if click.toggle {
            ClickMode::Toggle
        } else {
            ClickMode::Replace
        };
        self.table
            .update(cx, |d| d.selection.click(&d.rows, click.row, mode));
        self.selection_changed(cx);
        if click.count >= 2 && mode == ClickMode::Replace {
            self.open_row(click.row, cx);
        }
    }

    /// The selection changed: redraw now and tell listeners.
    pub(super) fn selection_changed(&mut self, cx: &mut Context<Self>) {
        self.selected = self.table.read(cx, |d| d.selection.len());
        cx.emit(ResourceTableEvent::SelectionChanged);
        cx.notify();
    }

    /// Moves the cursor by `delta` rows (`extend` selects the range) and scrolls it into view.
    pub fn move_cursor(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let moved = self
            .table
            .update(cx, |d| d.selection.move_cursor(&d.rows, delta, extend));
        if let Some(row) = moved {
            self.table.reveal_row(row, cx);
        }
        self.selection_changed(cx);
    }

    /// Puts the cursor on the first (`last == false`) or last row.
    fn go_to_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let row = self.table.update(cx, |d| {
            let row = if last {
                d.rows.len().checked_sub(1)
            } else {
                (!d.rows.is_empty()).then_some(0)
            };
            if let Some(row) = row {
                d.selection.go_to(&d.rows, row, false);
            }
            row
        });
        if let Some(row) = row {
            self.table.reveal_row(row, cx);
        }
        self.selection_changed(cx);
    }

    fn page(&self, cx: &gpui::App) -> isize {
        let visible = self.table.visible_rows(cx).len();
        isize::try_from(visible.saturating_sub(1).max(1)).unwrap_or(1)
    }

    /// Half of [`Self::page`], at least one row.
    fn half_page(&self, cx: &gpui::App) -> isize {
        (self.page(cx) / 2).max(1)
    }

    /// Opens row `row`'s detail: dispatches `resource::Open`.
    pub fn open_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(key) = self.row_key(row, cx) {
            self.open_object(key, cx);
        }
    }

    /// Opens the object `key` (wherever its row is now): dispatches `resource::Open`, its detail.
    /// On the CRD list (E07-S07) it dispatches `crd::OpenResources` instead: the table of the
    /// custom resources the definition defines (its detail is "Show Details" in the row menu).
    pub fn open_object(&mut self, key: ObjectKey, cx: &mut Context<Self>) {
        let target = self.resource_ref(key);
        if crate::crds::is_crd_kind(&self.kind.gvk) {
            let command = Command::CrdOpenResources {
                cluster: target.cluster,
                name: target.name.to_string(),
            };
            self.deps.dispatcher.dispatch(command, cx);
            return;
        }
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpen { target }, cx);
    }

    /// Copies row `row`'s name: dispatches `resource::CopyName`.
    pub fn copy_row_name(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(key) = self.row_key(row, cx) {
            self.copy_object_name(key, cx);
        }
    }

    /// Copies the name of the object `key` (wherever its row is now): dispatches
    /// `resource::CopyName`.
    pub fn copy_object_name(&mut self, key: ObjectKey, cx: &mut Context<Self>) {
        let target = self.resource_ref(key);
        self.deps
            .dispatcher
            .dispatch(Command::ResourceCopyName { target }, cx);
    }

    /// Asks for every row to be selected: dispatches `resource::SelectAll`.
    pub fn request_select_all(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceSelectAll {
            cluster: self.cluster.clone(),
            gvk: self.kind.gvk.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Selects every row (what `resource::SelectAll` does).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |d| d.selection.select_all(&d.rows));
        self.selection_changed(cx);
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |d| d.selection.clear());
        self.selection_changed(cx);
    }

    /// `resource::Open` ran for `target`, one of this table's rows: emits
    /// [`ResourceTableEvent::OpenDetail`] (the drawer itself is opened by `ResourceViews`).
    pub fn open_detail(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        cx.emit(ResourceTableEvent::OpenDetail(target));
    }

    /// The cursor row, if it is still listed.
    pub fn cursor_row(&self, cx: &gpui::App) -> Option<usize> {
        self.table.read(cx, |d| d.selection.cursor_index(&d.rows))
    }

    /// Sorts by column `id` (`descending` first when `true`), or by the store's default order
    /// with `None`, as a header click does; saves the layout.
    pub fn sort_by(&mut self, sort: Option<(ColumnId, bool)>, cx: &mut Context<Self>) {
        if self.table.update(cx, |d| d.layout.set_sort(sort)) {
            self.table.refresh(cx);
            self.apply_sort(cx);
            self.save_prefs(cx);
        }
    }

    /// Shows or hides column `id` (the column picker) and saves the layout.
    pub fn set_column_shown(&mut self, id: &ColumnId, shown: bool, cx: &mut Context<Self>) {
        let changed = self.table.update(cx, |d| d.layout.set_shown(id, shown));
        if changed {
            self.table.refresh(cx);
            // Hiding the sort column falls back to the default order.
            self.apply_sort(cx);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    pub(super) fn on_select_next(
        &mut self,
        _: &SelectNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(1, false, cx);
    }

    pub(super) fn on_select_previous(
        &mut self,
        _: &SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(-1, false, cx);
    }

    pub(super) fn on_extend_next(
        &mut self,
        _: &ExtendNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(1, true, cx);
    }

    pub(super) fn on_extend_previous(
        &mut self,
        _: &ExtendPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(-1, true, cx);
    }

    pub(super) fn on_select_first(
        &mut self,
        _: &SelectFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_end(false, cx);
    }

    pub(super) fn on_select_last(
        &mut self,
        _: &SelectLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_end(true, cx);
    }

    pub(super) fn on_page_down(
        &mut self,
        _: &SelectPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = self.page(cx);
        self.move_cursor(page, false, cx);
    }

    pub(super) fn on_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        self.move_cursor(-page, false, cx);
    }

    pub(super) fn on_half_page_down(
        &mut self,
        _: &SelectHalfPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let half = self.half_page(cx);
        self.move_cursor(half, false, cx);
    }

    pub(super) fn on_half_page_up(
        &mut self,
        _: &SelectHalfPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let half = self.half_page(cx);
        self.move_cursor(-half, false, cx);
    }

    pub(super) fn on_open(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.cursor_row(cx) {
            self.open_row(row, cx);
        }
    }

    pub(super) fn on_copy_name(&mut self, _: &CopyName, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.cursor_row(cx) {
            self.copy_row_name(row, cx);
        }
    }

    pub(super) fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.request_select_all(cx);
    }

    /// `escape` in the rows: clears the selection; with nothing selected it clears the filter,
    /// as k9s does (the filter and its stored value go, the chip with them).
    pub(super) fn on_clear(
        &mut self,
        _: &ClearSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nothing_selected = self.table.read(cx, |d| d.selection.is_empty());
        if nothing_selected && !self.filter.read(cx).text().is_empty() {
            self.filter.update(cx, |bar, cx| bar.clear(window, cx));
        } else {
            self.clear_selection(cx);
        }
    }
}
