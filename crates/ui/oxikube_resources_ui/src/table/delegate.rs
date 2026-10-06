//! [`RowsDelegate`]: the table's data as `oxikube_ui`'s [`TableDelegate`] sees it.
//!
//! It holds the rows (the store's sorted list, applied delta by delta), the [`ColumnLayout`],
//! the [`ColumnProvider`] that reads the cells, and the [`Selection`]. The table is virtualised,
//! so `render_td` runs only for the rows on screen: it reads one cell through the provider
//! (which borrows from the object) and draws it. Nothing here sorts, filters or allocates per
//! row beyond the cell's text. Cells go to the table as [`TextCell`]s (its fast path) from the
//! [`CellCache`], so a frame that scrolls or churns re-reads only the rows that changed.

use std::sync::Arc;

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, WeakEntity,
    Window, div, px,
};
use jiff::Timestamp;
use oxikube_app::ColumnProvider;
use oxikube_app::columns::Align;
use oxikube_app::store::{FeedState, ObjectKey, StoreObject};
use oxikube_ui::menu::{PopupMenu, PopupMenuItem};
use oxikube_ui::table::SortDirection;
use oxikube_ui::table::TextCell;
use oxikube_ui::{TableColumn, TableDelegate};

use super::cell_cache::CellCache;
use super::cells::{ToneColors, cell_element};
use super::layout::ColumnLayout;
use super::selection::Selection;
use super::states::{StateLabels, TableState, state_view};
use super::view::ResourceTable;
use crate::actions::ActionSource;

/// The data behind a [`ResourceTable`]'s rows. See the [`table`](crate::table) module docs.
pub struct RowsDelegate {
    /// The rows, in the store's order.
    pub(super) rows: Vec<Arc<StoreObject>>,
    /// The arranged columns.
    pub(crate) layout: ColumnLayout,
    /// Reads the cells.
    pub(super) provider: Arc<dyn ColumnProvider>,
    /// The selected rows.
    pub(super) selection: Selection,
    /// The feed's state, for the empty view.
    pub(super) state: FeedState,
    /// What the kind is called and where it is listed, for the state views ("No pods in …").
    pub(super) labels: StateLabels,
    /// The active in-app filter as the user reads it, for the filtered-empty state.
    pub(super) filter: Option<String>,
    /// Whether the failure detail of the state view is expanded.
    pub(super) details_open: bool,
    /// "Now" for ages, refreshed once per frame by the view.
    pub(super) now: Timestamp,
    /// The tone colours, refreshed once per frame by the view.
    pub(super) colors: Option<ToneColors>,
    /// The visible cells' text and tone, kept between frames.
    pub(super) cells: CellCache,
    /// The view, for the context menu's entries.
    pub(super) view: WeakEntity<ResourceTable>,
    /// The row actions of the context menu (none for a table without them).
    pub(super) actions: Option<ActionSource>,
    /// How many cells were drawn (virtualisation tests).
    #[cfg(test)]
    pub(super) rendered_cells: usize,
}

impl RowsDelegate {
    /// The row at `ix`.
    pub fn row(&self, ix: usize) -> Option<&Arc<StoreObject>> {
        self.rows.get(ix)
    }

    /// The rows.
    pub fn rows(&self) -> &[Arc<StoreObject>] {
        &self.rows
    }

    /// The column layout.
    pub fn layout(&self) -> &ColumnLayout {
        &self.layout
    }

    /// The selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The provider that reads the cells.
    pub fn provider(&self) -> &Arc<dyn ColumnProvider> {
        &self.provider
    }

    /// The feed's state.
    pub fn state(&self) -> &FeedState {
        &self.state
    }

    /// What the table shows now: loading, empty, filtered-empty, forbidden, unauthorized, an
    /// error, or rows (stale or not). See [`states`](super::states).
    pub fn table_state(&self) -> TableState {
        TableState::derive(&self.state, self.rows.len(), self.filter.as_deref())
    }

    /// What the state views name: the kind and where it is listed.
    pub fn labels(&self) -> &StateLabels {
        &self.labels
    }

    fn colors(&self, cx: &App) -> ToneColors {
        self.colors.unwrap_or_else(|| ToneColors::current(cx))
    }
}

impl TableDelegate for RowsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.layout.visible_len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let Some(column) = self.layout.visible(col_ix) else {
            return TableColumn::new(format!("missing-{col_ix}"), "");
        };
        let width = self
            .layout
            .width(&column.id)
            .unwrap_or_else(|| ColumnLayout::default_width(column));
        let mut out = TableColumn::new(column.id.to_string(), column.title.to_string())
            .min_width(px(40.))
            .width(px(width));
        if column.align == Align::Right {
            out = out.right();
        }
        match self.layout.sort() {
            Some((id, descending)) if *id == column.id => out.sorted(if *descending {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            }),
            _ => out.sortable(),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let colors = self.colors(cx);
        let cell = match (self.rows.get(row_ix), self.layout.visible(col_ix)) {
            (Some(row), Some(column)) => self.provider.cell(row, &column.id, self.now),
            _ => oxikube_app::Cell::empty(),
        };
        #[cfg(test)]
        {
            self.rendered_cells += 1;
        }
        cell_element(&cell, &colors, row_ix, col_ix)
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<TextCell> {
        let colors = self.colors(cx);
        let (row, column) = (self.rows.get(row_ix)?, self.layout.visible(col_ix)?);
        let (text, tone) = self.cells.get(row, &column.id, &*self.provider, self.now);
        #[cfg(test)]
        {
            self.rendered_cells += 1;
        }
        Some(TextCell::new(text).color(colors.of(tone)))
    }

    fn render_th(&mut self, col_ix: usize, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (id, title) = self
            .layout
            .visible(col_ix)
            .map(|c| (c.id.to_string(), c.title.to_string()))
            .unwrap_or_default();
        div()
            .debug_selector(move || format!("th-{id}"))
            .child(SharedString::from(title))
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        direction: SortDirection,
        _: &mut Window,
        _: &mut App,
    ) {
        let Some(id) = self.layout.visible(col_ix).map(|c| c.id.clone()) else {
            return;
        };
        let sort = match direction {
            SortDirection::Unsorted => None,
            SortDirection::Ascending => Some((id, false)),
            SortDirection::Descending => Some((id, true)),
        };
        self.layout.set_sort(sort);
    }

    fn move_column(&mut self, from: usize, to: usize, _: &mut Window, _: &mut App) {
        self.layout.move_visible(from, to);
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.table_state();
        state_view(&state, &self.labels, self.details_open, &self.view, cx)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.rows
            .get(row_ix)
            .is_some_and(|row| self.selection.contains_object(row))
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut App,
    ) -> PopupMenu {
        // The entries act on the object right-clicked, not on its row: the feed keeps moving
        // rows while the menu is open.
        let Some(key) = self.rows.get(row_ix).map(|row| row.key()) else {
            return menu;
        };
        let view = self.view.clone();
        let entry =
            |label: &'static str,
             run: fn(&mut ResourceTable, ObjectKey, &mut gpui::Context<ResourceTable>)| {
                let view = view.clone();
                let key = key.clone();
                PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    view.update(cx, |table, cx| run(table, key.clone(), cx))
                        .ok();
                })
            };
        let menu = menu
            .item(entry("Open", ResourceTable::open_object))
            .item(entry("Copy Name", ResourceTable::copy_object_name))
            .separator()
            .item(entry("Select All", |table, _, cx| {
                table.request_select_all(cx)
            }));
        let Some(source) = &self.actions else {
            return menu;
        };
        // The actions work on the selection when the row is part of it, else on the row alone.
        let targets = if self.selection.contains(&key) {
            self.selection
                .in_row_order(&self.rows)
                .into_iter()
                .map(|key| source.target(key))
                .collect()
        } else {
            vec![source.target(key)]
        };
        source.append(menu, targets, view)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        match (self.rows.get(row_ix), self.layout.visible(col_ix)) {
            (Some(row), Some(column)) => self
                .provider
                .cell(row, &column.id, self.now)
                .display()
                .to_owned(),
            _ => String::new(),
        }
    }
}
