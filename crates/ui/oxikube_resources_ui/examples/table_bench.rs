//! Frame cost of the resource table: 10 000 pods through the real store, churning, scrolled
//! frame by frame.
//!
//! `cargo run -p oxikube_resources_ui --profile release-fast --example table_bench`
//!
//! Runs on GPUI's headless platform with the host's real text system (as `oxikube
//! --perf-scenario` does; no present, no GPU time), with testkit fakes behind a
//! real `ClusterSessionManager` and `ResourceStore`: each frame the feed delivers a batch of
//! modified pods (the budget's churn, 1 % of the pods every 5 s, is about two pods a frame at
//! 120 Hz; the bench uses ten), the table applies it, scrolls and draws. It measures what the
//! view adds (delta apply, cell reads for the visible rows, element building, layout), not GPU
//! frame time: `oxikube --perf` against kind is the budget measurement (docs/PERFORMANCE.md).
//! Prints the whole frame (feed delivery, delta apply, the headless scheduler, draw) and the draw
//! alone, p50 / p95 / p99 / max, and the coalesced redraws per frame. The draw is the frame's
//! cost in the app; the whole-frame figure also holds the headless context's own redraw when the
//! coalesced notify lands (a test-mode context draws dirty windows as effects flush), so with
//! churn it counts two draws per frame where the app draws once per vsync.
//!
//! `OXIKUBE_BENCH_TYPE=pod-0123` (E07-S04) types that into the filter bar while the feed churns
//! and the table scrolls: one more character every six frames (the first applies at once, the
//! rest after the bar's debounce), then back out one character at a time.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, px, size};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{ClockPort, ClusterContext, Delta, DeltaBatch, SourceId};
use oxikube_resources_ui::table::{ResourceTable, ResourceTableDeps, store_runtime};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, Timeline, pod,
};
use oxikube_workspace::CommandDispatcher;

const PODS: usize = 10_000;
const FRAMES: usize = 300;
/// Pods modified per frame (`OXIKUBE_BENCH_CHURN` overrides).
fn churn_per_frame() -> usize {
    std::env::var("OXIKUBE_BENCH_CHURN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}
const FRAME: Duration = Duration::from_micros(8_333);

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

/// Rows scrolled per frame (`OXIKUBE_BENCH_STEP` overrides): 3 is a fast wheel or trackpad
/// fling at 120 Hz; a full page per frame re-shapes every visible cell every frame.
fn step() -> usize {
    std::env::var("OXIKUBE_BENCH_STEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// The text typed into the filter bar during the run, if any.
fn typed() -> Option<String> {
    std::env::var("OXIKUBE_BENCH_TYPE")
        .ok()
        .filter(|t| !t.is_empty())
}

/// How many frames pass between two keystrokes.
const KEY_EVERY: usize = 6;

fn pod_named(i: usize, version: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns-{}", i % 20))
        .name(format!("pod-{i:05}"))
        .restarts(u32::try_from(version).unwrap_or(0))
        .build();
    r.meta.resource_version = Some(version.to_string().into());
    r
}

fn main() {
    let context = ContextName::new("bench");
    let cluster = ClusterId::new("/bench/kubeconfig", &context);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let mut timeline = Timeline::new().ok_at(
        Duration::ZERO,
        DeltaBatch::from_deltas(vec![Delta::Restarted(
            (0..PODS).map(|i| pod_named(i, 1)).collect(),
        )]),
    );
    let churn = churn_per_frame();
    for frame in 1..=FRAMES {
        if churn == 0 {
            break;
        }
        let batch = (0..churn)
            .map(|k| Delta::Applied(pod_named((frame * 97 + k * 1009) % PODS, frame + 1)))
            .collect();
        timeline = timeline.ok_at(
            FRAME * u32::try_from(frame).unwrap_or(0),
            DeltaBatch::from_deltas(batch),
        );
    }
    ports.resources.script().watch.push_ok(timeline.keep_open());
    let clock = Arc::new(FakeClockPort::default());
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");

    let mut cx =
        oxikube_testkit::headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    let kind = ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["list", "watch"]),
        namespaced: true,
    };
    let window = cx
        .open_window(size(px(1280.), px(800.)), |window, cx| {
            oxikube_ui::init(cx);
            oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
            oxikube_runtime::init_deterministic(cx);
            let clock: Arc<dyn ClockPort> = clock.clone();
            let deps = ResourceTableDeps {
                sessions: sessions.clone(),
                stores: Arc::new(ResourceStores::new(store_runtime(clock, cx))),
                columns: Arc::new(CoreColumns::new()),
                state: Arc::new(FakeStatePort::new()),
                dispatcher: Rc::new(Ignore),
                actions: None,
            };
            cx.new(|cx| ResourceTable::new(cluster.clone(), kind, deps, window, cx))
        })
        .expect("a window");
    cx.run_until_parked();
    let view = cx
        .update_window(window.into(), |root, _, _| root.downcast::<ResourceTable>())
        .expect("the window")
        .expect("the table");
    let rows = cx.update(|cx| view.read(cx).read_rows(cx, |d| d.rows().len()));
    assert_eq!(rows, PODS, "the store listed every pod");
    if let Some(text) = typed() {
        println!("typing {text:?} into the filter bar while the feed churns");
    }

    let recorder = Arc::new(oxikube_runtime::perf::Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let mut frame_ms = Vec::with_capacity(FRAMES);
    let mut draw_ms = Vec::with_capacity(FRAMES);
    let typing = typed();
    for frame in 0..FRAMES {
        if let Some(text) = &typing
            && frame % KEY_EVERY == 0
        {
            // Type forward through the text, then back out.
            let n = frame / KEY_EVERY;
            let len = text.chars().count();
            let k = if n <= len {
                n
            } else {
                (2 * len).saturating_sub(n)
            };
            let current: String = text.chars().take(k).collect();
            cx.update_window(window.into(), |_, window, cx| {
                view.update(cx, |table, cx| table.set_filter_text(&current, window, cx));
            })
            .expect("type");
        }
        let started = Instant::now();
        ports.resources.clock().advance(FRAME);
        cx.run_until_parked();
        cx.advance_clock(FRAME);
        cx.run_until_parked();
        cx.update(|cx| {
            let table = view.read(cx).table().clone();
            table.scroll_to_row(frame * step() % PODS, cx);
        });
        let drawing = Instant::now();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .expect("draw");
        draw_ms.push(drawing.elapsed().as_secs_f64() * 1000.0);
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let notifies = recorder.notifies();
    let (columns, visible, cell_us) = cx.update(|cx| {
        let view = view.read(cx);
        let visible = view.table().visible_rows(cx);
        view.read_rows(cx, |d| {
            let ids = d.layout().visible_ids();
            let now = jiff::Timestamp::now();
            let started = Instant::now();
            let mut cells = 0usize;
            for row in &d.rows()[visible.clone()] {
                for id in &ids {
                    std::hint::black_box(d.provider().cell(row, id, now));
                    cells += 1;
                }
            }
            let per_cell = started.elapsed().as_secs_f64() * 1e6 / cells.max(1) as f64;
            (ids.len(), visible.len(), per_cell)
        })
    });
    println!("{columns} columns x {visible} visible rows; one cell read costs {cell_us:.2} us");
    frame_ms.sort_by(f64::total_cmp);
    draw_ms.sort_by(f64::total_cmp);
    let pct = |v: &[f64], q: usize| v[(v.len() * q / 100).min(v.len() - 1)];
    let at = |q: usize| pct(&frame_ms, q);
    println!(
        "resource table_bench: {PODS} pods, {churn} modified and {} rows scrolled per frame, {FRAMES} frames: \
         frame ms p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}; coalesced redraws {notifies} ({:.2}/frame)",
        step(),
        at(50),
        at(95),
        at(99),
        frame_ms[frame_ms.len() - 1],
        notifies as f64 / FRAMES as f64,
    );
    println!(
        "  of which draw (render, layout, paint of the window): p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}",
        pct(&draw_ms, 50),
        pct(&draw_ms, 95),
        pct(&draw_ms, 99),
        draw_ms[draw_ms.len() - 1],
    );
}
