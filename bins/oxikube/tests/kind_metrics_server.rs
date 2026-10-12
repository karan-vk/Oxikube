//! The metrics path end to end on kind with metrics-server (E13-S13, tier 1): the app on the
//! real init order (its main window headless, kind connected) against the pinned metrics-server
//! `cargo xtask kind-up` installs.
//!
//! * The Pods and Nodes tables a user opens from the sidebar fill their CPU and memory columns
//!   from metrics-server, and the live service's ring buffers (the sparklines' data) fill with
//!   one reading per scrape.
//! * The cluster overview (`cluster::ShowOverview`, the palette's "Cluster Overview") has
//!   populated health tiles, live usage on the capacity bars and top lists, and, with
//!   metrics-server only, no graph data: the graphs say `metrics unavailable: <reason>`.
//!   With the Prometheus stack tier on, the graphs are `kind_metrics_stack`'s to check.
//!
//! The absent case (no `metrics.k8s.io` at all) is `oxikube_app`'s `kind_smoke::metrics_absent`,
//! which needs a client that hides the API from the adapter. Nothing is installed: the test's own
//! pod lives in its `oxi-test-<rand>` namespace, deleted at the end.
//!
//! `cargo test -p oxikube --features integration --test kind_metrics_server` with
//! `OXIKUBE_TEST_CONTEXT` set; `OXIKUBE_TEST_METRICS_SERVER=0` skips it
//! (`oxikube_testkit::integration::metrics`). Real I/O wakes GPUI tasks from Tokio threads, so
//! the tests allow parking and poll with short real-time sleeps.
#![cfg(feature = "integration")]

mod kind_common;

use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use oxikube::app_state::AppState;
use oxikube_app::ColumnId;
use oxikube_app::metrics::live::UsagePoint;
use oxikube_app::store::FeedState;
use oxikube_overview_ui::overview::HistoryState;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::metrics::{MetricsTier, tier_context, wait_metrics_api};
use oxikube_workspace::ClusterTab;
use oxikube_workspace::sidebar::SidebarPanel;

use kind_common::{Launched, create_pod, launch_with, open_overview};

/// metrics-server scrapes every 15 s and reports a new pod after one or two scrapes; a ring
/// needs two distinct scrapes.
const METRICS_DEADLINE: Duration = Duration::from_secs(150);

/// Polls every five seconds, the fastest the setting allows, so the ring fills in two scrapes.
fn settings() -> serde_json::Value {
    serde_json::json!({ "metrics": { "poll_interval_secs": 5 } })
}

/// Runs the app until `done`; panics after [`METRICS_DEADLINE`] (waits for a scrape, not an
/// assertion that is retried).
fn wait_for_metrics(
    vcx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let started = Instant::now();
    loop {
        vcx.run_until_parked();
        if done(vcx) {
            return;
        }
        assert!(
            started.elapsed() < METRICS_DEADLINE,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The display text of `column` on the row named `name` (in `namespace`, for a namespaced kind).
fn cell(
    vcx: &mut VisualTestContext,
    table: &Entity<ResourceTable>,
    namespace: Option<&str>,
    name: &str,
    column: &str,
) -> Option<String> {
    vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let row = d
                .rows()
                .iter()
                .find(|o| &*o.meta().name == name && o.meta().namespace.as_deref() == namespace)?;
            Some(
                d.provider()
                    .cell(row, &ColumnId::new(column), Timestamp::now())
                    .display()
                    .to_owned(),
            )
        })
    })
}

/// Whether `text` is whole digits followed by `unit`.
fn is_whole(text: &str, unit: &str) -> bool {
    text.strip_suffix(unit)
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn is_cpu(text: &str) -> bool {
    is_whole(text, "m")
}

fn is_memory(text: &str) -> bool {
    ["Ki", "Mi", "Gi"].iter().any(|unit| is_whole(text, unit))
}

/// Opens the Nodes table from the sidebar, as a user does, and waits for it to list.
fn open_nodes_table(
    vcx: &mut VisualTestContext,
    tab: &Entity<ClusterTab>,
) -> Entity<ResourceTable> {
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let panel = vcx
        .update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the sidebar");
    vcx.update(|_, cx| panel.update(cx, |panel, cx| panel.activate("nodes/nodes", cx)));
    kind_common::wait_value(vcx, "the Nodes table to open and list", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|t| {
                    let table = t.read(cx);
                    &*table.gvk().kind == "Node"
                        && table.read_rows(cx, |d| {
                            d.state() == &FeedState::Ready && !d.rows().is_empty()
                        })
                })
        })
    })
}

/// A node no fake-node suite created.
fn real_node(vcx: &mut VisualTestContext, table: &Entity<ResourceTable>) -> String {
    vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.rows()
                .iter()
                .map(|o| o.meta().name.to_string())
                .find(|n| !n.starts_with("oxi-fake-"))
                .expect("a real node")
        })
    })
}

/// A ring holds readings in time order, none of them a gap.
fn assert_ring(what: &str, history: &[UsagePoint]) {
    assert!(
        history.windows(2).all(|w| w[0].at < w[1].at),
        "{what} ring is in time order: {history:?}"
    );
    assert!(
        history
            .iter()
            .all(|p| p.cpu_quantity().is_some() && p.memory_quantity().is_some()),
        "{what} ring holds readings, not gaps: {history:?}"
    );
}

#[gpui::test]
fn the_pods_and_nodes_tables_populate_from_metrics_server_and_the_rings_fill(
    cx: &mut TestAppContext,
) {
    let Some(context) = tier_context(MetricsTier::MetricsServer) else {
        return;
    };
    wait_metrics_api(&context, Duration::from_secs(180)).expect("metrics-server serves");
    cx.executor().allow_parking();
    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "busy");

    let Launched {
        mut vcx,
        tab,
        cluster,
        table: pods,
        ..
    } = launch_with(cx, &context, settings());

    // The Pods table: the test pod's CPU and memory, read by its column provider as it draws.
    wait_for_metrics(&mut vcx, "the test pod's usage in the Pods table", |vcx| {
        cell(vcx, &pods, Some(ns.name()), "busy", "memory").is_some_and(|m| is_memory(&m))
    });
    let cpu = cell(&mut vcx, &pods, Some(ns.name()), "busy", "cpu").expect("the row");
    assert!(is_cpu(&cpu), "pod cpu {cpu:?}");
    // metrics-server's own pod uses something: the reading is a real one, not a placeholder.
    let server_memory = vcx.update(|_, cx| {
        pods.read(cx).read_rows(cx, |d| {
            d.rows()
                .iter()
                .find(|o| {
                    o.meta().namespace.as_deref() == Some("kube-system")
                        && o.meta().name.starts_with("metrics-server-")
                })
                .map(|row| {
                    d.provider()
                        .cell(row, &ColumnId::new("memory"), Timestamp::now())
                        .display()
                        .to_owned()
                })
        })
    });
    assert!(
        server_memory.as_deref().is_some_and(is_memory) && server_memory.as_deref() != Some("0Mi"),
        "metrics-server's memory {server_memory:?}"
    );

    // The ring behind the pod's sparkline: one reading per scrape, oldest first. Pods are polled
    // while the Pods table is the pane's active item.
    let usage = vcx.update(|_, cx| AppState::global(cx).services().live_metrics.usage(&cluster));
    wait_for_metrics(&mut vcx, "two scrapes in the pod's ring", |_| {
        usage.pod_history(ns.name(), "busy").len() >= 2
    });
    assert_ring("pod", &usage.pod_history(ns.name(), "busy"));

    // The Nodes table: CPU, memory and their share of what the node can allocate.
    let nodes = open_nodes_table(&mut vcx, &tab);
    let node = real_node(&mut vcx, &nodes);
    wait_for_metrics(&mut vcx, "the node's usage in the Nodes table", |vcx| {
        cell(vcx, &nodes, None, &node, "cpu").is_some_and(|c| is_cpu(&c))
    });
    let memory = cell(&mut vcx, &nodes, None, &node, "memory").expect("the node row");
    assert!(is_memory(&memory), "node memory {memory:?}");
    for column in ["cpu-percent", "memory-percent"] {
        let percent = cell(&mut vcx, &nodes, None, &node, column).expect("the node row");
        let value: f64 = percent
            .strip_suffix('%')
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("{column} {percent:?}"));
        // A share, not a placeholder: a big node idling at a few millicores rounds to `0%`.
        assert!((0.0..=100.0).contains(&value), "{column} {percent}");
    }

    // The node's ring, filled while the Nodes table is shown.
    wait_for_metrics(&mut vcx, "two scrapes in the node's ring", |_| {
        usage.node_history(&node).len() >= 2
    });
    assert_ring("node", &usage.node_history(&node));
}

#[gpui::test]
fn the_overview_has_populated_tiles_and_usage_and_no_graphs_without_prometheus(
    cx: &mut TestAppContext,
) {
    let Some(context) = tier_context(MetricsTier::MetricsServer) else {
        return;
    };
    wait_metrics_api(&context, Duration::from_secs(180)).expect("metrics-server serves");
    cx.executor().allow_parking();
    let Launched {
        mut vcx,
        tab,
        cluster,
        ..
    } = launch_with(cx, &context, settings());
    let page = open_overview(&mut vcx, &tab, &cluster);

    wait_for_metrics(&mut vcx, "live usage on the overview", |vcx| {
        vcx.update(|_, cx| {
            let shown = page.read(cx).shown();
            shown.lines[0].usage.is_some() && !shown.top.pods_cpu.is_empty()
        })
    });
    let shown = vcx.update(|_, cx| page.read(cx).shown().clone());
    // The tiles count what the cluster holds: numbers, not "loading" or "no access".
    let tile = |id: &str| {
        shown
            .tiles
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("tile {id}: {:?}", shown.tiles))
            .clone()
    };
    let ready: usize = tile("nodes-ready").value.parse().expect("a node count");
    assert!(ready >= 1, "{:?}", tile("nodes-ready"));
    let running: usize = tile("pods-running").value.parse().expect("a pod count");
    assert!(running >= 1, "{:?}", tile("pods-running"));
    // Live usage from metrics-server, against allocatable; no `metrics unavailable` notice.
    assert_eq!(shown.notice, None);
    let [cpu, memory, _pods] = shown.lines;
    for line in [cpu, memory] {
        assert!(
            line.usage.is_some_and(|u| u > 0) && line.allocatable.is_some_and(|a| a > 0),
            "{line:?}"
        );
        assert!(line.usage_fill().is_some_and(|f| f > 0.0), "{line:?}");
    }
    assert!(!shown.top.pods_memory.is_empty() && !shown.top.namespaces_cpu.is_empty());
    assert!(
        shown
            .top
            .pods_memory
            .iter()
            .all(|row| is_memory(&row.value)),
        "{:?}",
        shown.top.pods_memory
    );

    if MetricsTier::PrometheusStack.is_enabled() {
        eprintln!("the Prometheus stack is installed: kind_metrics_stack checks the graphs");
        return;
    }
    // metrics-server only: the graphs have nothing to draw and say why, never an empty zero line.
    let mut state = HistoryState::Detecting;
    wait_for_metrics(&mut vcx, "the graphs' verdict", |vcx| {
        state = vcx.update(|_, cx| page.read(cx).history().state().clone());
        !matches!(state, HistoryState::Detecting | HistoryState::Loading)
    });
    assert!(matches!(state, HistoryState::Unavailable(_)), "{state:?}");
    let text = vcx.update(|_, cx| page.read(cx).history().unavailable_text());
    assert!(
        text.as_deref()
            .is_some_and(|t| t.starts_with("metrics unavailable: ")),
        "{text:?}"
    );
    assert!(
        vcx.update(|_, cx| page.read(cx).history().sparks().is_none()),
        "no graph data without Prometheus"
    );
}
