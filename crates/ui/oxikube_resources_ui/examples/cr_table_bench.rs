//! Frame cost of a custom resource table (E12-S17): 5 000 objects of a CRD on the server-side
//! Table feed, the server's printer columns, churning, scrolled frame by frame.
//!
//! `cargo run -p oxikube_resources_ui --profile release-fast --example cr_table_bench`
//!
//! The same harness as `table_bench` (GPUI's headless platform with the host's text system,
//! testkit fakes behind a real `ClusterSessionManager` and `ResourceStore`), but the kind is a
//! custom resource: its rows are Table rows (cells as the server rendered them) read through the
//! `TableColumns` provider, which is the path every CRD list takes. Each frame the feed delivers
//! modified rows, the table applies them, scrolls and draws. Prints the whole frame and the draw
//! alone, p50 / p95 / p99 / max. `OXIKUBE_BENCH_ROWS`, `OXIKUBE_BENCH_FRAMES`,
//! `OXIKUBE_BENCH_CHURN` and `OXIKUBE_BENCH_STEP` override the defaults.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, px, size};
use oxikube_app::ClusterSessionManager;
use oxikube_app::CoreColumns;
use oxikube_app::store::ResourceStores;
use oxikube_domain::ObjectMeta;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{
    ClockPort, ClusterContext, Delta, DeltaBatch, SourceId, TableBatch, TableColumn, TableRow,
    TableSource,
};
use oxikube_resources_ui::table::{ResourceTable, ResourceTableDeps, store_runtime};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, Timeline,
};
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

const FRAME: Duration = Duration::from_micros(8_333);

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

/// The sample CRD's printer columns (`kubectl get widgets -o wide`).
fn columns() -> Arc<[TableColumn]> {
    [
        ("Name", "string", "name", 0),
        ("Size", "string", "", 0),
        ("Replicas", "integer", "", 0),
        ("Phase", "string", "", 0),
        ("Age", "date", "", 0),
        ("Owner", "string", "", 1),
    ]
    .into_iter()
    .map(|(name, ty, format, priority)| TableColumn {
        name: name.to_owned(),
        column_type: ty.to_owned(),
        format: format.to_owned(),
        description: String::new(),
        priority,
    })
    .collect()
}

fn row(i: usize, version: usize) -> TableRow {
    let name = format!("widget-{i:05}");
    let mut meta = ObjectMeta::named(name.as_str());
    meta.namespace = Some(format!("ns-{}", i % 20).into());
    meta.uid = Some(format!("uid-{i}").into());
    meta.resource_version = Some(version.to_string().into());
    meta.creation = "2026-01-01T00:00:00Z".parse().ok();
    let size = ["small", "medium", "large"][i % 3];
    let phase = ["Ready", "Pending", "Failed"][(i + version) % 3];
    TableRow {
        cells: vec![
            json!(name),
            json!(size),
            json!(version % 10),
            json!(phase),
            json!("7d12h"),
            json!(format!("team-{}", i % 7)),
        ],
        meta: Some(meta),
        object: None,
    }
}

fn main() {
    let rows = env("OXIKUBE_BENCH_ROWS", 5_000);
    let frames = env("OXIKUBE_BENCH_FRAMES", 300);
    let churn = env("OXIKUBE_BENCH_CHURN", 10);
    let step = env("OXIKUBE_BENCH_STEP", 3);
    let context = ContextName::new("bench");
    let cluster = ClusterId::new("/bench/kubeconfig", &context);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    let kind = ResourceKind {
        gvk: Gvk::new("test.oxikube.dev", "v1", "Widget"),
        preferred: true,
        plural: "widgets".into(),
        singular: "widget".into(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["list", "watch"]),
        namespaced: true,
    };
    ports.discovery.set_kinds([kind.clone()]);
    let mut timeline = Timeline::new().ok_at(
        Duration::ZERO,
        TableBatch {
            columns: Some(columns()),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(
                (0..rows).map(|i| row(i, 1)).collect(),
            )]),
            source: TableSource::Server,
        },
    );
    for frame in 1..=frames {
        if churn == 0 {
            break;
        }
        let batch = (0..churn)
            .map(|k| Delta::Applied(row((frame * 97 + k * 1009) % rows, frame + 1)))
            .collect();
        timeline = timeline.ok_at(
            FRAME * u32::try_from(frame).unwrap_or(0),
            TableBatch {
                columns: None,
                rows: DeltaBatch::from_deltas(batch),
                source: TableSource::Server,
            },
        );
    }
    ports
        .tables
        .script()
        .table_feed
        .push_ok(timeline.keep_open());
    let clock = Arc::new(FakeClockPort::default());
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");

    let mut cx =
        oxikube_testkit::headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
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
    let listed = cx.update(|cx| view.read(cx).read_rows(cx, |d| d.rows().len()));
    assert_eq!(listed, rows, "the store listed every row");
    let headers: Vec<String> = cx.update(|cx| {
        view.read(cx).read_rows(cx, |d| {
            let layout = d.layout();
            (0..layout.visible_len())
                .filter_map(|ix| layout.visible(ix))
                .map(|c| c.title.to_string())
                .collect()
        })
    });
    println!("columns: {}", headers.join(" | "));

    let mut frame_ms = Vec::with_capacity(frames);
    let mut draw_ms = Vec::with_capacity(frames);
    for frame in 0..frames {
        let started = Instant::now();
        ports.tables.clock().advance(FRAME);
        cx.run_until_parked();
        cx.advance_clock(FRAME);
        cx.run_until_parked();
        cx.update(|cx| {
            let table = view.read(cx).table().clone();
            table.scroll_to_row(frame * step % rows, cx);
        });
        let drawing = Instant::now();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .expect("draw");
        draw_ms.push(drawing.elapsed().as_secs_f64() * 1000.0);
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    frame_ms.sort_by(f64::total_cmp);
    draw_ms.sort_by(f64::total_cmp);
    let pct = |v: &[f64], q: usize| v[(v.len() * q / 100).min(v.len() - 1)];
    println!(
        "cr_table_bench: {rows} custom resources (Table feed), {churn} modified and {step} rows \
         scrolled per frame, {frames} frames: frame ms p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}",
        pct(&frame_ms, 50),
        pct(&frame_ms, 95),
        pct(&frame_ms, 99),
        frame_ms[frame_ms.len() - 1],
    );
    println!(
        "  of which draw: p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}",
        pct(&draw_ms, 50),
        pct(&draw_ms, 95),
        pct(&draw_ms, 99),
        draw_ms[draw_ms.len() - 1],
    );
}
