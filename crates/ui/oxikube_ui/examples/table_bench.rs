//! Micro benchmark: scroll a 10 000-row [`Table`] frame by frame and report frame cost.
//!
//! `cargo run -p oxikube_ui --profile release-fast --example table_bench`
//!
//! Runs on GPUI's test platform (no GPU, and the test text system), so it measures what the
//! wrapper adds on top of gpui-component: element building, layout and the delegate calls for the
//! visible rows. It is a regression check for the wrapper, not a frame-time budget measurement:
//! those need `oxikube --perf` against the real table (E07, ADR 0013). Prints rows built per frame
//! (must stay in the dozens, never thousands) and frame time mean / p95 / max.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use gpui::{
    App, Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
};
use oxikube_ui::{Table, TableColumn, TableDelegate, TableHandle};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

const ROWS: usize = 10_000;
const FRAMES: usize = 300;

struct Synthetic {
    built: Rc<Cell<usize>>,
}

impl TableDelegate for Synthetic {
    fn columns_count(&self, _: &App) -> usize {
        6
    }
    fn rows_count(&self, _: &App) -> usize {
        ROWS
    }
    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        TableColumn::new(format!("c{col_ix}"), format!("Column {col_ix}"))
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        self.built.set(self.built.get() + 1);
        div().child(format!("pod-{row}-{col}"))
    }
}

struct Bench {
    table: TableHandle<Synthetic>,
}

impl Render for Bench {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Table::new(&self.table))
    }
}

fn main() {
    let built = Rc::new(Cell::new(0));
    let mut cx = TestAppContext::single();
    cx.update(oxikube_ui::init);
    let delegate_built = built.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| Bench {
        table: TableHandle::new(
            Synthetic {
                built: delegate_built,
            },
            window,
            cx,
        ),
    });
    cx.run_until_parked();

    let mut frame_ms = Vec::with_capacity(FRAMES);
    built.set(0);
    for frame in 0..FRAMES {
        let started = Instant::now();
        view.update_in(cx, |bench, _, cx| {
            bench.table.scroll_to_row(frame * 31 % ROWS, cx)
        });
        cx.run_until_parked();
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    frame_ms.sort_by(|a, b| a.total_cmp(b));
    let mean = frame_ms.iter().sum::<f64>() / frame_ms.len() as f64;
    let p95 = frame_ms[(frame_ms.len() * 95 / 100).min(frame_ms.len() - 1)];
    let max = frame_ms[frame_ms.len() - 1];
    println!(
        "table_bench: {ROWS} rows x 6 cols, {FRAMES} scroll frames: {:.0} cells built/frame, \
         frame ms mean {mean:.2} p95 {p95:.2} max {max:.2}",
        built.get() as f64 / FRAMES as f64
    );
}
