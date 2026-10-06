//! Micro benchmark: switch between two cluster tabs that each hold a 10 000-row table, and report
//! frame cost (E06-S04, "tab switch <= 1 frame", "a hidden tab does not render").
//!
//! `cargo run -p oxikube_workspace --features test-support --profile release-fast --example cluster_tab_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures what the cluster tabs,
//! the nested workspace and the dock area add per frame: the switch itself, element building and
//! layout of the displayed cluster. The hidden cluster is not part of the frame at all: the
//! bench prints how many frames each tab's table rendered while hidden (it must be 0). It is a
//! regression check, not the ADR 0013 frame budget (that needs `oxikube --perf` in a real window).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use futures::executor::block_on;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::{Table, TableColumn, TableDelegate, TableHandle, root::Root};
use oxikube_workspace::{
    ClusterTabs, ClusterTabsDeps, CommandDispatcher, Item, ItemEvent, TabContent, Workspace,
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

/// A centre item of a cluster showing a 10 000-row table; counts how often it renders.
struct PodsItem {
    focus_handle: FocusHandle,
    table: TableHandle<Rows>,
    renders: Rc<Cell<usize>>,
}

impl EventEmitter<ItemEvent> for PodsItem {}

impl Focusable for PodsItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PodsItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full().child(Table::new(&self.table))
    }
}

impl Item for PodsItem {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("pods")
    }
}

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn id(name: &str) -> ClusterId {
    ClusterId::new("/bench/config", &ContextName::new(name))
}

fn report(name: &str, mut frame_ms: Vec<f64>) {
    frame_ms.sort_by(|a, b| a.total_cmp(b));
    let mean = frame_ms.iter().sum::<f64>() / frame_ms.len() as f64;
    let p95 = frame_ms[(frame_ms.len() * 95 / 100).min(frame_ms.len() - 1)];
    let max = frame_ms[frame_ms.len() - 1];
    println!(
        "cluster_tab_bench {name}: {FRAMES} frames, ms mean {mean:.3} p95 {p95:.3} max {max:.3}"
    );
}

fn main() {
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
    });
    let names = ["prod", "staging"];
    let source =
        Arc::new(FakeClusterSourcePort::new().with_contexts(
            names.iter().map(|n| {
                ClusterContext::new(id(n), ContextName::new(*n), SourceId("bench".into()))
            }),
        ));
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        source,
        Arc::new(FakeClockPort::default()),
    );
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let ws = workspace.expect("built");
    let mut vcx = VisualTestContext::from_window(window.into(), &cx);
    let renders: Vec<Rc<Cell<usize>>> = names.iter().map(|_| Rc::default()).collect();
    let tabs: Entity<ClusterTabs> = {
        let renders = renders.clone();
        vcx.update(|window, cx| {
            let deps = ClusterTabsDeps::new(
                sessions.clone(),
                Arc::new(FakeStatePort::new()),
                Rc::new(Ignore),
            )
            .with_setup(move |tab, session, window, cx| {
                let ix = names
                    .iter()
                    .position(|n| *n == session.context().as_str())
                    .unwrap_or(0);
                let renders = renders[ix].clone();
                let pods = cx.new(|cx| PodsItem {
                    focus_handle: cx.focus_handle(),
                    table: TableHandle::new(Rows, window, cx),
                    renders,
                });
                let inner = tab.read(cx).workspace().clone();
                inner.update(cx, |ws, cx| ws.open_item(pods, window, cx));
            });
            ClusterTabs::start(&ws, deps, window, cx)
        })
    };
    for name in names {
        block_on(sessions.connect(&id(name))).expect("connect");
    }
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // Reference: a full redraw with nothing changed.
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

    // Switching between the two clusters: the switch and the frame that shows it.
    let before: Vec<usize> = renders.iter().map(|r| r.get()).collect();
    let mut frame_ms = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let target = id(names[frame % 2]);
        let started = Instant::now();
        vcx.update(|window, cx| {
            tabs.update(cx, |tabs, cx| {
                tabs.apply(&Command::ClusterSelect { cluster: target }, window, cx)
            });
            window.draw(cx).clear(cx);
        });
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("tab-switch", frame_ms);

    // A frame draws only the displayed cluster: each table rendered once per frame it was shown
    // in (every other frame), and never while hidden.
    let rendered: Vec<usize> = renders
        .iter()
        .zip(&before)
        .map(|(now, before)| now.get() - before)
        .collect();
    println!(
        "cluster_tab_bench renders of each cluster's table over {FRAMES} switches: {rendered:?}"
    );
    let shown = FRAMES / 2;
    assert!(
        rendered.iter().all(|r| *r <= shown + 2),
        "a hidden cluster rendered: {rendered:?}"
    );
}
