//! Micro benchmark: resize a dock (the dock-divider drag path) and switch tabs with a heavy item
//! mounted, and report frame cost.
//!
//! `cargo run -p oxikube_workspace --features test-support --profile release-fast --example workspace_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures what the workspace and
//! the dock area add per frame: element building, layout, the tab bars, the docks. The heavy item
//! is a 10 000-row virtualised table. It is a regression check, not the ADR 0013 frame budget
//! (that needs `oxikube --perf` in a real window).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::Instant;

use gpui::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use oxikube_ui::{Table, TableColumn, TableDelegate, TableHandle, Unscaled, root::Root};
use oxikube_workspace::{
    DockPosition, Item, ItemEvent, TabContent, Workspace, test_support::TestPanel,
};

const ROWS: usize = 10_000;
const FRAMES: usize = 300;

struct Rows;

impl TableDelegate for Rows {
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
        div().child(format!("pod-{row}-{col}"))
    }
}

/// A centre item showing a 10 000-row table.
struct HeavyItem {
    focus_handle: FocusHandle,
    title: &'static str,
    table: TableHandle<Rows>,
}

impl EventEmitter<ItemEvent> for HeavyItem {}

impl Focusable for HeavyItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HeavyItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Table::new(&self.table))
    }
}

impl Item for HeavyItem {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title)
    }
}

fn heavy(title: &'static str, window: &mut Window, cx: &mut App) -> gpui::Entity<HeavyItem> {
    cx.new(|cx| HeavyItem {
        focus_handle: cx.focus_handle(),
        title,
        table: TableHandle::new(Rows, window, cx),
    })
}

fn report(name: &str, mut frame_ms: Vec<f64>) {
    frame_ms.sort_by(|a, b| a.total_cmp(b));
    let mean = frame_ms.iter().sum::<f64>() / frame_ms.len() as f64;
    let p95 = frame_ms[(frame_ms.len() * 95 / 100).min(frame_ms.len() - 1)];
    let max = frame_ms[frame_ms.len() - 1];
    println!(
        "workspace_bench {name}: {FRAMES} frames, ms mean {mean:.3} p95 {p95:.3} max {max:.3}"
    );
}

fn main() {
    let mut cx = TestAppContext::single();
    cx.update(oxikube_ui::init);
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let ws = workspace.expect("built");
    let mut vcx = VisualTestContext::from_window(window.into(), &cx);
    let (a, b) = vcx.update(|window, cx| {
        let left = TestPanel::build(DockPosition::Left, "tree", cx);
        let bottom = TestPanel::build(DockPosition::Bottom, "logs", cx);
        let first = heavy("pods", window, cx);
        let second = heavy("deployments", window, cx);
        ws.update(cx, |ws, cx| {
            ws.add_panel(left, window, cx);
            ws.add_panel(bottom, window, cx);
            let a = ws.open_item(first, window, cx);
            let b = ws.open_item(second, window, cx);
            (a, b)
        })
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // Reference: a full redraw with nothing changed (the floor the two scenarios add to).
    let mut frame_ms = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let started = Instant::now();
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("idle-redraw", frame_ms);

    // Dragging the left dock's divider: one resize per frame, as the drag handler does.
    let mut frame_ms = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let size = 200. + (frame % 200) as f32;
        let started = Instant::now();
        vcx.update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.resize_dock(DockPosition::Left, Unscaled(size), window, cx)
            });
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("dock-resize", frame_ms);

    // Switching between the two heavy tabs: the switch and the frame that shows it.
    let mut frame_ms = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let item = if frame % 2 == 0 { a } else { b };
        let started = Instant::now();
        vcx.update(|window, cx| {
            ws.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("tab-switch", frame_ms);
}
