//! The metrics path end to end against real metrics stacks (E13-S13, tier 2: the nightly's
//! `metrics-stack` job): the app on the real init order (its main window headless, kind
//! connected) against the pinned kube-prometheus-stack and VictoriaMetrics that
//! `cargo xtask kind-metrics-stack` installs into the kind TEST cluster.
//!
//! * Auto-detection finds the stack's Prometheus with no setting, classifies it as
//!   kube-prometheus, and the PromQL port answers through it.
//! * The detail panels' Metrics tab queries (the query catalogue's kube-prometheus profile)
//!   return series for a known pod (the API server's static pod) and for the node.
//! * The cluster overview's graphs and sparklines have series.
//! * VictoriaMetrics, set as a manual endpoint (`prometheus.path` + `path_prefix`, a namespace
//!   detection does not look in), answers range queries and the panels' queries under its prefix.
//!
//! Each tier is off unless turned on (`OXIKUBE_TEST_PROMETHEUS_STACK=1`,
//! `OXIKUBE_TEST_VICTORIA_METRICS=1`; `oxikube_testkit::integration::metrics`), so the PR suite and
//! a developer's cluster without the stack skip it. Only the stack's own namespaces are read;
//! nothing is written. Waits are for readiness (a scrape landing, detection ending), never retries
//! of an assertion; a failure names the PromQL and prints the stack's pods and events.
//!
//! `OXIKUBE_TEST_CONTEXT=kind-oxikube OXIKUBE_TEST_PROMETHEUS_STACK=1 OXIKUBE_TEST_VICTORIA_METRICS=1
//! cargo test -p oxikube --features integration --test kind_metrics_stack`
#![cfg(feature = "integration")]

mod kind_common;

use std::future::Future;
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::{
    ChartData, HistoryError, HistorySubject, ProviderStatus, SubjectKind, charts_for,
};
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command as BusCommand;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metric_query::RangePreset;
use oxikube_overview_ui::ClusterOverview;
use oxikube_overview_ui::overview::HistoryState;
use oxikube_ports::RangePlan;
use oxikube_testkit::integration::metrics::{
    KUBE_PROMETHEUS_NAMESPACE, KUBE_PROMETHEUS_SERVICE, MetricsTier, VICTORIA_NAMESPACE,
    VICTORIA_PATH_PREFIX, assert_samples, assert_series, namespace_diagnostics, tier_context,
    victoria_service_path,
};
use oxikube_workspace::ClusterTab;

use kind_common::{Launched, launch, launch_with};

/// A fresh stack needs a few scrapes (15 s apart) before a rate has two samples; detection probes
/// each candidate once per connect.
const STACK_DEADLINE: Duration = Duration::from_secs(300);

/// Runs the app until `get` gives a value, moving GPUI's test clock so the views' own timers
/// (the overview's graph refresh) fire; panics after [`STACK_DEADLINE`] with the stack's pods and
/// events. Readiness only.
fn ready<T>(
    vcx: &mut VisualTestContext,
    context: &str,
    namespace: &str,
    what: &str,
    mut get: impl FnMut(&mut VisualTestContext) -> Option<T>,
) -> T {
    let started = Instant::now();
    loop {
        vcx.run_until_parked();
        if let Some(value) = get(vcx) {
            return value;
        }
        assert!(
            started.elapsed() < STACK_DEADLINE,
            "timed out after {STACK_DEADLINE:?} waiting for {what}\n{}",
            namespace_diagnostics(context, namespace)
        );
        vcx.executor().advance_clock(Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn status_of(vcx: &mut VisualTestContext, cluster: &ClusterId) -> std::sync::Arc<ProviderStatus> {
    vcx.update(|_, cx| {
        AppState::global(cx)
            .services()
            .metrics
            .provider
            .status(cluster)
    })
}

/// Runs `fut` on the app's Tokio runtime and waits for it (the test thread is the app's).
fn on_app_runtime<T>(vcx: &mut VisualTestContext, fut: impl Future<Output = T>) -> T {
    let handle = vcx
        .update(|_, cx| oxikube_runtime::handle(cx))
        .expect("the app runs on Tokio");
    handle.block_on(fut)
}

/// Waits for detection (or the manual endpoint's check) to name a provider, and asserts its
/// kind. A timeout prints the last status (its verdict and the services it looked at).
fn provider_found(
    vcx: &mut VisualTestContext,
    context: &str,
    namespace: &str,
    cluster: &ClusterId,
    kind: MetricsProviderKind,
) -> String {
    let started = Instant::now();
    let status = loop {
        vcx.run_until_parked();
        let status = status_of(vcx, cluster);
        if status.provider().is_some() {
            break status;
        }
        assert!(
            started.elapsed() < STACK_DEADLINE,
            "no provider after {STACK_DEADLINE:?}: {:?}\nprobes: {:?}\n{}",
            status.unavailable_text(),
            status.probes,
            namespace_diagnostics(context, namespace)
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    let provider = status.provider().expect("found");
    assert_eq!(provider.kind, kind, "{provider:?}");
    provider.target.to_string()
}

/// The name of the kind cluster's (only) real node.
fn node_name(context: &str) -> String {
    let out = Command::new("kubectl")
        .args(["--context", context, "get", "nodes", "-o", "name"])
        .output()
        .expect("kubectl get nodes");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("node/"))
        .find(|n| !n.starts_with("oxi-fake-"))
        .expect("a real node")
        .to_owned()
}

/// Fetches the Metrics tab's charts of `subject` over the last 15 minutes, as the panel does.
fn fetch_charts(
    vcx: &mut VisualTestContext,
    cluster: &ClusterId,
    subject: &HistorySubject,
) -> Vec<(&'static str, Result<ChartData, HistoryError>)> {
    let history = vcx.update(|_, cx| AppState::global(cx).services().metrics.history.clone());
    let charts = charts_for(subject);
    let results = on_app_runtime(
        vcx,
        history.fetch(cluster, &charts, RangePreset::Minutes15, Timestamp::now()),
    );
    charts.iter().map(|c| c.id).zip(results).collect()
}

/// Waits until the CPU and memory charts of `subject` have readings, then asserts every chart
/// loaded and that those two hold real, non-negative samples.
fn assert_panel_charts(
    vcx: &mut VisualTestContext,
    context: &str,
    namespace: &str,
    cluster: &ClusterId,
    subject: &HistorySubject,
) {
    let what = format!("the CPU and memory graphs of {subject:?}");
    let charts = ready(vcx, context, namespace, &what, |vcx| {
        let charts = fetch_charts(vcx, cluster, subject);
        let has = |id: &str| {
            charts
                .iter()
                .any(|(chart, data)| *chart == id && data.as_ref().is_ok_and(|d| !d.is_empty()))
        };
        (has("cpu") && has("memory")).then_some(charts)
    });
    for (id, data) in &charts {
        let data = data
            .as_ref()
            .unwrap_or_else(|e| panic!("{subject:?} chart {id}: {}", e.text()));
        if matches!(*id, "cpu" | "memory") {
            for series in data.series.iter().filter(|s| s.has_data()) {
                assert_samples(
                    &format!("{subject:?} {id} {}", series.name),
                    series.samples.iter().map(|s| s.v),
                );
            }
        }
    }
}

#[gpui::test]
fn detection_finds_and_classifies_the_kube_prometheus_stack_and_queries_through_it(
    cx: &mut TestAppContext,
) {
    let Some(context) = tier_context(MetricsTier::PrometheusStack) else {
        return;
    };
    cx.executor().allow_parking();
    // No `prometheus` setting: automatic detection.
    let Launched {
        mut vcx, cluster, ..
    } = launch(cx, &context);

    let target = provider_found(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        &cluster,
        MetricsProviderKind::KubePrometheus,
    );
    assert!(
        target.starts_with(&format!(
            "{KUBE_PROMETHEUS_NAMESPACE}/{KUBE_PROMETHEUS_SERVICE}:"
        )),
        "{target}"
    );
    let status = status_of(&mut vcx, &cluster);
    assert!(status.unavailable_text().is_none(), "{status:?}");

    // The PromQL port reaches it through the API server's service proxy.
    let promql = vcx.update(|_, cx| AppState::global(cx).services().metrics.promql.clone());
    let query = "up{job=\"kube-state-metrics\"}";
    let up = ready(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        "kube-state-metrics to be scraped",
        |vcx| {
            let series = on_app_runtime(vcx, promql.query_instant(&cluster, query, None))
                .expect("a query through the service proxy");
            (!series.is_empty()).then_some(series)
        },
    );
    assert_series("kube-state-metrics up", query, &up);
}

#[gpui::test]
fn the_metrics_tab_queries_return_series_for_a_known_pod_and_the_node(cx: &mut TestAppContext) {
    let Some(context) = tier_context(MetricsTier::PrometheusStack) else {
        return;
    };
    cx.executor().allow_parking();
    let Launched {
        mut vcx, cluster, ..
    } = launch(cx, &context);
    provider_found(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        &cluster,
        MetricsProviderKind::KubePrometheus,
    );

    // The API server's static pod: there since the cluster started, named after its node.
    let node = node_name(&context);
    let pod = HistorySubject::new(
        SubjectKind::Pod,
        Some("kube-system"),
        &format!("kube-apiserver-{node}"),
        [],
    )
    .expect("a pod subject");
    assert_panel_charts(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        &cluster,
        &pod,
    );
    let node = HistorySubject::new(SubjectKind::Node, None, &node, []).expect("a node subject");
    assert_panel_charts(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        &cluster,
        &node,
    );
}

fn open_overview(
    vcx: &mut VisualTestContext,
    tab: &Entity<ClusterTab>,
    cluster: &ClusterId,
) -> Entity<ClusterOverview> {
    let bus = vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the bus");
    let shown = futures::executor::block_on(bus.dispatch(
        BusCommand::ClusterShowOverview {
            cluster: cluster.clone(),
        },
        DispatchContext::new(Initiator::Ui, "kind test"),
    ));
    assert!(shown.is_ok(), "{shown:?}");
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let mut found = None;
    kind_common::wait(vcx, "the overview to open", |vcx| {
        found = vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<ClusterOverview>()
                .into_iter()
                .next()
        });
        found.is_some()
    });
    found.expect("the overview")
}

#[gpui::test]
fn the_overview_graphs_have_series_from_the_stack(cx: &mut TestAppContext) {
    let Some(context) = tier_context(MetricsTier::PrometheusStack) else {
        return;
    };
    cx.executor().allow_parking();
    let Launched {
        mut vcx,
        tab,
        cluster,
        ..
    } = launch(cx, &context);
    let page = open_overview(&mut vcx, &tab, &cluster);

    let sparks = ready(
        &mut vcx,
        &context,
        KUBE_PROMETHEUS_NAMESPACE,
        "the overview's graphs to have readings",
        |vcx| {
            vcx.update(|_, cx| {
                let history = page.read(cx).history();
                if let HistoryState::Failed(reason) = history.state() {
                    panic!("the overview's graphs failed: {reason}");
                }
                let sparks = history.sparks()?;
                let has =
                    |points: &[oxikube_ui::chart::Point]| points.iter().any(|p| p.v.is_finite());
                (*history.state() == HistoryState::Ready && has(&sparks.cpu) && has(&sparks.memory))
                    .then(|| sparks.clone())
            })
        },
    );
    assert_samples("overview CPU", sparks.cpu.iter().map(|p| p.v));
    assert_samples("overview memory", sparks.memory.iter().map(|p| p.v));
    assert!(sparks.range.0 < sparks.range.1, "{:?}", sparks.range);
    // The live tiles do not wait on Prometheus: metrics-server feeds them.
    let notice = vcx.update(|_, cx| page.read(cx).shown().notice.clone());
    assert_eq!(notice, None);
}

#[gpui::test]
fn victoria_metrics_answers_as_a_manual_endpoint_under_its_path_prefix(cx: &mut TestAppContext) {
    let Some(context) = tier_context(MetricsTier::VictoriaMetrics) else {
        return;
    };
    cx.executor().allow_parking();
    let settings = serde_json::json!({
        "prometheus": {
            "kind": "victoria_metrics",
            "path": victoria_service_path(),
            "path_prefix": VICTORIA_PATH_PREFIX,
        }
    });
    let Launched {
        mut vcx, cluster, ..
    } = launch_with(cx, &context, settings);

    let target = provider_found(
        &mut vcx,
        &context,
        VICTORIA_NAMESPACE,
        &cluster,
        MetricsProviderKind::VictoriaMetrics,
    );
    assert!(target.contains(&victoria_service_path()), "{target}");

    // A range query under the prefix: VictoriaMetrics scrapes itself every 15 s.
    let promql = vcx.update(|_, cx| AppState::global(cx).services().metrics.promql.clone());
    let query = "up{job=\"victoriametrics\"}";
    let series = ready(
        &mut vcx,
        &context,
        VICTORIA_NAMESPACE,
        "VictoriaMetrics to hold its own scrapes",
        |vcx| {
            let plan = RangePlan::ending_at(Timestamp::now(), Duration::from_secs(600));
            let series = on_app_runtime(vcx, promql.query_range(&cluster, query, plan.range))
                .expect("a range query through the path prefix");
            series.iter().any(|s| s.points.len() >= 2).then_some(series)
        },
    );
    assert_series("VictoriaMetrics up", query, &series);

    // The Metrics tab of a pod through it: it scrapes the kubelets' cAdvisor too.
    let node = node_name(&context);
    let pod = HistorySubject::new(
        SubjectKind::Pod,
        Some("kube-system"),
        &format!("kube-apiserver-{node}"),
        [],
    )
    .expect("a pod subject");
    assert_panel_charts(&mut vcx, &context, VICTORIA_NAMESPACE, &cluster, &pod);
}
