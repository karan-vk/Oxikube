//! A counting delegate and a harness view for table tests.

use crate::table::{Table, TableColumn, TableDelegate, TableHandle};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

/// Records which cells the table asked for.
#[derive(Clone, Default)]
pub struct Counting(Rc<RefCell<CountingInner>>);

#[derive(Default)]
struct CountingInner {
    cells: usize,
    rows: BTreeSet<usize>,
}

impl Counting {
    pub fn cells_rendered(&self) -> usize {
        self.0.borrow().cells
    }
    pub fn max_row_rendered(&self) -> usize {
        self.0.borrow().rows.last().copied().unwrap_or(0)
    }
    pub fn distinct_rows_rendered(&self) -> usize {
        self.0.borrow().rows.len()
    }
}

/// Synthetic rows: `rows` rows by three columns.
pub struct Synthetic {
    pub rows: usize,
    counts: Counting,
}

impl TableDelegate for Synthetic {
    fn columns_count(&self, _: &App) -> usize {
        3
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows
    }
    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        // Design-time width: the table applies the UI zoom.
        TableColumn::new(format!("c{col_ix}"), format!("Column {col_ix}")).width(px(150.))
    }
    fn render_th(&mut self, col_ix: usize, _: &mut Window, _: &mut App) -> impl IntoElement {
        // Column 0's header body is tagged so tests can measure the width the table gave it.
        div()
            .id(col_ix)
            .size_full()
            .when(col_ix == 0, |th| th.debug_selector(|| "th-0".into()))
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        let mut inner = self.counts.0.borrow_mut();
        inner.cells += 1;
        inner.rows.insert(row_ix);
        // Row 0 column 0's cell body is tagged so tests can measure the row height.
        div()
            .size_full()
            .child(format!("r{row_ix}c{col_ix}"))
            .when(row_ix == 0 && col_ix == 0, |td| {
                td.debug_selector(|| "td-0-0".into())
            })
    }
}

pub struct Harness {
    pub table: TableHandle<Synthetic>,
    pub counts: Counting,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Table::new(&self.table))
    }
}

/// Opens a window holding a table of `rows` synthetic rows.
pub fn harness(cx: &mut TestAppContext, rows: usize) -> (Entity<Harness>, &mut VisualTestContext) {
    cx.update(crate::init);
    let counts = Counting::default();
    let counts_for_view = counts.clone();
    cx.add_window_view(move |window, cx| {
        let delegate = Synthetic {
            rows,
            counts: counts_for_view,
        };
        let table = TableHandle::new(delegate, window, cx);
        Harness { table, counts }
    })
}
