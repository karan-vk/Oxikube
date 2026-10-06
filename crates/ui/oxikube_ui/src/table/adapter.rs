//! Bridges our [`TableDelegate`] to gpui-component's.

use super::column::{ColumnAlign, SortDirection};
use super::delegate::TableDelegate;
use super::events::{RowClick, TableEvent, TableEvents};
use super::text_cell::text_cell;
use super::widths::ColumnWidths;
use crate::menu::PopupMenu;
use crate::size::UiScale;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Div, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use gpui_component::ActiveTheme as _;
use gpui_component::table::{
    Column, ColumnSort, TableDelegate as LibDelegate, TableState as LibState,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// Wraps a delegate so it satisfies gpui-component's trait. Each method forwards, reborrowing the
/// library's `Context` as the `App` our trait takes.
///
/// gpui-component 0.7 stores a column's alignment but never applies it, so the adapter does:
/// `aligns` caches each column's alignment as the library reads the column definitions (once per
/// create/refresh), and cells and headers are laid out with it.
///
/// Column widths are scaled by the UI zoom as the library reads them (see [`ColumnWidths`]).
///
/// Rows get a click listener that reports the click with its modifiers ([`RowClick`]) and the
/// owner's selection background ([`TableDelegate::row_selected`]); header sorts are reported as
/// [`TableEvent::SortChanged`]. The library has no event for either, so they go out through
/// `events`.
pub(super) struct Adapter<D> {
    pub(super) delegate: D,
    aligns: RefCell<Vec<ColumnAlign>>,
    pub(super) widths: Rc<ColumnWidths>,
    events: Entity<TableEvents>,
    /// Whether headers sort ([`TableOptions::sortable`](super::TableOptions)).
    sortable: bool,
}

impl<D> Adapter<D> {
    pub(super) fn new(
        delegate: D,
        widths: Rc<ColumnWidths>,
        events: Entity<TableEvents>,
        sortable: bool,
    ) -> Self {
        Self {
            delegate,
            aligns: RefCell::new(Vec::new()),
            widths,
            events,
            sortable,
        }
    }

    fn align(&self, col_ix: usize) -> ColumnAlign {
        self.aligns
            .borrow()
            .get(col_ix)
            .copied()
            .unwrap_or_default()
    }
}

/// Places `cell` in a full-size flex box, vertically centred, justified per `align`.
pub(super) fn aligned(align: ColumnAlign, cell: impl IntoElement) -> Div {
    let base = div().size_full().flex().items_center();
    match align {
        ColumnAlign::Left => base.justify_start(),
        ColumnAlign::Center => base.justify_center(),
        ColumnAlign::Right => base.justify_end(),
    }
    .child(cell)
}

impl<D: TableDelegate> Adapter<D> {
    /// Sorts by column `col_ix` in `direction`: the delegate re-sorts, the owner hears of it.
    fn sort_to(
        &mut self,
        col_ix: usize,
        direction: SortDirection,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.delegate.perform_sort(col_ix, direction, window, cx);
        self.events.update(cx, |_, cx| {
            cx.emit(TableEvent::SortChanged {
                column: col_ix,
                direction,
            })
        });
    }
}

impl<D: TableDelegate> LibDelegate for Adapter<D> {
    fn columns_count(&self, cx: &App) -> usize {
        self.delegate.columns_count(cx)
    }

    fn rows_count(&self, cx: &App) -> usize {
        self.delegate.rows_count(cx)
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let column = self.delegate.column(col_ix, cx);
        let mut aligns = self.aligns.borrow_mut();
        if aligns.len() <= col_ix {
            aligns.resize(col_ix + 1, ColumnAlign::default());
        }
        aligns[col_ix] = column.align;
        let library = column.to_library(
            UiScale::new(crate::size::current_scale()),
            self.widths.user_width(col_ix),
        );
        self.widths.record_supplied(col_ix, library.width);
        library
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) -> impl IntoElement {
        let align = self.align(col_ix);
        if let Some(cell) = self.delegate.text_cell(row_ix, col_ix, window, cx) {
            let width = self.widths.supplied(col_ix);
            return text_cell(cell, align, width, row_ix, col_ix, window, cx);
        }
        aligned(align, self.delegate.render_td(row_ix, col_ix, window, cx)).into_any_element()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) -> impl IntoElement {
        let align = self.align(col_ix);
        let header = aligned(align, self.delegate.render_th(col_ix, window, cx)).into_any_element();
        // The library sorts only from its small sort icon; the whole header label sorts too.
        div()
            .id(ElementId::NamedInteger("th-sort".into(), col_ix as u64))
            .size_full()
            .when(self.sortable, |th| {
                th.on_click(cx.listener(move |state, _, window, cx| {
                    let adapter = state.delegate_mut();
                    let Some(current) = adapter.delegate.column(col_ix, cx).sort else {
                        return;
                    };
                    // The library's cycle: unsorted, descending, ascending, unsorted.
                    let next = match current {
                        SortDirection::Unsorted => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                        SortDirection::Ascending => SortDirection::Unsorted,
                    };
                    adapter.sort_to(col_ix, next, window, cx);
                    // Re-read the columns so every header shows the new sort state.
                    state.refresh(cx);
                    cx.notify();
                }))
            })
            .child(header)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) -> Stateful<Div> {
        let selected = self.delegate.row_selected(row_ix, cx);
        let events = self.events.clone();
        div()
            .id(ElementId::NamedInteger("row".into(), row_ix as u64))
            .when(selected, |tr| tr.bg(cx.theme().tokens.table_active))
            // Runs before the library's own click handler (listeners run in the order added).
            .on_click(move |event, _, cx| {
                let click = RowClick::from_click(row_ix, event);
                events.update(cx, |_, cx| cx.emit(TableEvent::RowClicked(click)));
            })
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) -> PopupMenu {
        self.delegate.context_menu(row_ix, menu, window, cx)
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) {
        {
            let mut aligns = self.aligns.borrow_mut();
            if col_ix < aligns.len() && to_ix < aligns.len() {
                let align = aligns.remove(col_ix);
                aligns.insert(to_ix, align);
            }
        }
        self.widths.moved(col_ix, to_ix);
        self.delegate.move_column(col_ix, to_ix, window, cx);
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) {
        self.sort_to(col_ix, SortDirection::from(sort), window, cx);
    }

    fn render_empty(
        &mut self,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) -> impl IntoElement {
        self.delegate.render_empty(window, cx)
    }

    fn loading(&self, cx: &App) -> bool {
        self.delegate.loading(cx)
    }

    fn has_more(&self, cx: &App) -> bool {
        self.delegate.has_more(cx)
    }

    fn load_more_threshold(&self) -> usize {
        self.delegate.load_more_threshold()
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<LibState<Adapter<D>>>) {
        self.delegate.load_more(window, cx);
    }

    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<LibState<Adapter<D>>>,
    ) {
        self.delegate
            .visible_rows_changed(visible_range, window, cx);
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        self.delegate.cell_text(row_ix, col_ix, cx)
    }
}
