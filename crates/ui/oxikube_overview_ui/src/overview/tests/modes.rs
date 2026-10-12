//! The three modes the story names, on the live view: metrics-server only (bars with usage and
//! top lists, no graphs), Prometheus available (graphs and sparklines from the catalogue's
//! queries) and metrics unavailable (a visible reason with the way to the settings; the tiles and
//! allocation bars stay).

use std::sync::Arc;

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::MetricsKind;
use oxikube_app::cluster_overview::HistoryAvailability;
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::command::Command;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::{MetricsOutcome, PromqlSeries, PromqlValue};
use oxikube_testkit::{FakePromqlPort, FakeQueryCataloguePort, node, pod};
use oxikube_ui::chart::ChartState;
use parking_lot::Mutex;

use super::{FixedSource, Fixture, Setup, cluster, node_sample, pod_sample};
use crate::overview::{HistoryDeps, HistoryState};

fn objects() -> Vec<oxikube_domain::Resource> {
    vec![
        node().name("n1").allocatable("4", "16Gi", "110").build(),
        node().name("n2").allocatable("4", "16Gi", "110").build(),
        pod()
            .namespace("shop")
            .name("web-0")
            .running()
            .node("n1")
            .requests("1", "1Gi")
            .build(),
        pod()
            .namespace("shop")
            .name("web-1")
            .running()
            .node("n2")
            .requests("1", "1Gi")
            .build(),
        pod()
            .namespace("data")
            .name("db-0")
            .running()
            .node("n2")
            .requests("2", "4Gi")
            .build(),
        pod().namespace("data").name("etl").pending().build(),
    ]
}

fn live_metrics() -> Vec<(MetricsKind, MetricsOutcome)> {
    vec![
        (
            MetricsKind::Nodes,
            MetricsOutcome::Available(vec![
                node_sample("n1", 900, 2048),
                node_sample("n2", 2100, 6144),
            ]),
        ),
        (
            MetricsKind::Pods,
            MetricsOutcome::Available(vec![
                pod_sample("shop", "web-0", 400, 300),
                pod_sample("shop", "web-1", 300, 280),
                pod_sample("data", "db-0", 1500, 3000),
            ]),
        ),
    ]
}

#[gpui::test]
fn metrics_server_only_draws_usage_and_top_lists_but_no_graphs(cx: &mut TestAppContext) {
    let mut f = Fixture::open(
        cx,
        Setup {
            objects: objects(),
            metrics: Some(live_metrics()),
            ..Setup::default()
        },
    );
    f.tick();
    let shown = f.read(|o| o.shown().clone());
    let [cpu, memory, pods] = shown.lines;
    assert_eq!(cpu.allocatable, Some(8_000));
    assert_eq!(cpu.requests, 4_000);
    assert_eq!(cpu.usage, Some(3_000), "the nodes' usage, summed");
    assert_eq!(memory.usage, Some(8192 * 1024 * 1024));
    assert_eq!(pods.requests, 3, "the pending pod is on no node");
    assert_eq!(shown.notice, None);
    let pods_cpu: Vec<_> = shown
        .top
        .pods_cpu
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(pods_cpu, ["data/db-0", "shop/web-0", "shop/web-1"]);
    let namespaces: Vec<_> = shown
        .top
        .namespaces_cpu
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(namespaces, ["data", "shop"]);
    assert!(
        f.drawn("overview-capacity-cpu-usage"),
        "the usage bar is drawn"
    );
    assert!(f.drawn("overview-top-pods-cpu-0"));
    assert!(
        !f.drawn("overview-history"),
        "no history metrics: no graphs card at all"
    );
    assert_eq!(f.read(|o| o.history().state().clone()), HistoryState::Off);
}

fn range(values: &[f64]) -> Vec<PromqlSeries> {
    let now = Timestamp::now().as_second();
    vec![PromqlSeries {
        labels: Default::default(),
        points: values
            .iter()
            .enumerate()
            .map(|(i, v)| PromqlValue {
                ts: Timestamp::from_second(now - 600 + 60 * i as i64).expect("a time"),
                value: *v,
            })
            .collect(),
    }]
}

#[gpui::test]
fn with_prometheus_the_graphs_and_sparklines_come_from_the_catalogue(cx: &mut TestAppContext) {
    let promql = Arc::new(FakePromqlPort::new());
    promql
        .script()
        .query_range
        .push_ok(range(&[1.0, 2.0, 3.0]))
        .push_ok(range(&[4.0, 4.0, 4.0]))
        .push_ok(range(&[1e9, 2e9, 3e9]))
        .push_ok(range(&[4e9, 4e9, 4e9]));
    let catalogue = Arc::new(FakeQueryCataloguePort::new());
    let history = HistoryDeps {
        source: Arc::new(FixedSource(Mutex::new(HistoryAvailability::Ready {
            provider: MetricsProviderKind::KubePrometheus,
        }))),
        promql: promql.clone(),
        catalogue: catalogue.clone(),
    };
    let mut f = Fixture::open(
        cx,
        Setup {
            objects: objects(),
            metrics: Some(live_metrics()),
            history: Some(history),
            ..Setup::default()
        },
    );
    f.tick();
    assert_eq!(f.read(|o| o.history().state().clone()), HistoryState::Ready);
    assert_eq!(
        promql.recorded_calls().len(),
        4,
        "CPU and memory, used and requested"
    );
    assert!(
        catalogue
            .recorded_calls()
            .iter()
            .all(|c| c.provider == MetricsProviderKind::KubePrometheus)
    );
    let sparks = f
        .read(|o| o.history().sparks().cloned())
        .expect("sparklines");
    assert_eq!(sparks.cpu.len(), 3);
    let (cpu, memory) = f.read(|o| {
        (
            o.history().cpu_chart().clone(),
            o.history().memory_chart().clone(),
        )
    });
    let states = f
        .vcx
        .update(|_, cx| (cpu.read(cx).state(), memory.read(cx).state()));
    assert_eq!(states, (ChartState::Plot, ChartState::Plot));
    assert!(f.drawn("overview-history"));
    assert!(f.drawn("overview-capacity"));
}

#[gpui::test]
fn metrics_unavailable_is_a_visible_reason_and_the_counts_stay(cx: &mut TestAppContext) {
    let history = HistoryDeps {
        source: Arc::new(FixedSource(Mutex::new(HistoryAvailability::Unavailable {
            reason: "no Prometheus-compatible service was found".into(),
        }))),
        promql: Arc::new(FakePromqlPort::new()),
        catalogue: Arc::new(FakeQueryCataloguePort::new()),
    };
    let mut f = Fixture::open(
        cx,
        Setup {
            objects: objects(),
            metrics: Some(vec![
                (
                    MetricsKind::Nodes,
                    MetricsOutcome::Unavailable(MissingReason::NotInstalled),
                ),
                (
                    MetricsKind::Pods,
                    MetricsOutcome::Unavailable(MissingReason::NotInstalled),
                ),
            ]),
            history: Some(history),
            ..Setup::default()
        },
    );
    f.tick();
    let shown = f.read(|o| o.shown().clone());
    let notice = shown.notice.clone().expect("a notice");
    assert!(
        notice.starts_with("metrics unavailable: metrics-server is not installed"),
        "{notice}"
    );
    assert_eq!(shown.lines[0].usage, None, "no usage, never a zero");
    assert_eq!(
        shown.lines[0].requests, 4_000,
        "the allocation does not need metrics"
    );
    assert!(shown.top.pods_cpu.is_empty());
    assert_eq!(
        shown
            .tiles
            .iter()
            .find(|t| t.id == "pods-running")
            .map(|t| t.value.as_str()),
        Some("3")
    );
    assert!(matches!(
        f.read(|o| o.history().state().clone()),
        HistoryState::Unavailable(_)
    ));
    assert!(f.drawn("overview-history-notice"));
    assert!(!f.drawn("overview-capacity-cpu-usage"));

    f.click("overview-live-notice");
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterShowMetricsSettings { cluster: cluster() }],
        "the notice leads to the cluster's metrics settings"
    );
}
