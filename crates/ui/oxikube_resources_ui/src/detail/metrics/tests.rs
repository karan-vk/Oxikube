//! The Metrics tab over a scripted `PromqlPort` and the testkit's placeholder catalogue:
//! `#[gpui::test]`s through the real drawer, store and session manager. No cluster, no HTTP.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::metrics_history::HistorySource;
use oxikube_app::{HistoryAvailability, MetricsHistoryService};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{MetricKind, MetricsProviderKind, OxiResult, RangePreset};
use oxikube_ports::{PromqlError, PromqlPort, PromqlSeries, PromqlValue, TimeRange};
use oxikube_testkit::FakeQueryCataloguePort;
use oxikube_ui::chart::ChartState;
use parking_lot::Mutex;
use tokio::sync::{Notify, broadcast};

use super::{MetricsHistory, MetricsState};
use crate::detail::tests::fixture::{Detail, pod_ref, web_pod};
use crate::detail::{DetailTab, DetailView};
use crate::table::tests::fixture::cluster;

const KUBE: MetricsProviderKind = MetricsProviderKind::KubePrometheus;

/// Scripted availability that can change under a running view.
struct Source {
    now: Mutex<HistoryAvailability>,
    changed: broadcast::Sender<()>,
}

impl Source {
    fn new(now: HistoryAvailability) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            changed: broadcast::channel(8).0,
        })
    }

    fn set(&self, now: HistoryAvailability) {
        *self.now.lock() = now;
        self.changed.send(()).ok();
    }
}

impl HistorySource for Source {
    fn availability(&self, _: &ClusterId) -> HistoryAvailability {
        self.now.lock().clone()
    }

    fn changes(&self, _: &ClusterId) -> BoxStream<'static, ()> {
        futures::stream::unfold(self.changed.subscribe(), |mut rx| async move {
            match rx.recv().await {
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Some(((), rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }
}

/// Answers by the placeholder query text (`fake:<provider>:<scope>:<kind>:<window>s`).
#[derive(Default)]
struct Prom {
    answers: Mutex<Vec<(String, Result<Vec<PromqlSeries>, PromqlError>)>>,
    calls: Mutex<Vec<(String, TimeRange)>>,
    /// While set, queries wait for `release` (a slow Prometheus).
    gated: AtomicBool,
    release: Notify,
    in_flight: AtomicUsize,
    /// While set, a query panics (a bug in the fetch).
    panics: AtomicBool,
}

struct InFlight<'a>(&'a Prom);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Prom {
    fn answer(&self, needle: &str, result: Result<Vec<PromqlSeries>, PromqlError>) {
        let mut answers = self.answers.lock();
        answers.retain(|(n, _)| n != needle);
        answers.push((needle.to_owned(), result));
    }

    fn queries(&self) -> usize {
        self.calls.lock().len()
    }
}

#[async_trait]
impl PromqlPort for Prom {
    async fn is_available(&self, _: &ClusterId) -> OxiResult<bool> {
        Ok(true)
    }

    async fn query_instant(
        &self,
        _: &ClusterId,
        _: &str,
        _: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>> {
        Ok(Vec::new())
    }

    async fn query_range(
        &self,
        _: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>> {
        self.calls.lock().push((query.to_owned(), range));
        assert!(!self.panics.load(Ordering::SeqCst), "the query blew up");
        if self.gated.load(Ordering::SeqCst) {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            let _in_flight = InFlight(self);
            self.release.notified().await;
        }
        self.answers
            .lock()
            .iter()
            .find(|(needle, _)| query.contains(needle.as_str()))
            .map_or(Ok(Vec::new()), |(_, result)| {
                result.clone().map_err(Into::into)
            })
    }
}

fn series(labels: &[(&str, &str)], values: &[f64]) -> PromqlSeries {
    let now = Timestamp::now();
    PromqlSeries {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        points: values
            .iter()
            .enumerate()
            .map(|(i, v)| PromqlValue {
                ts: now - Duration::from_secs(60 * (values.len() - i) as u64),
                value: *v,
            })
            .collect(),
    }
}

/// A pod `shop/web-0` with two containers, their usage, requests and limits.
fn two_containers(prom: &Prom) {
    prom.answer(
        "container:cpu_usage",
        Ok(vec![
            series(&[("container", "app")], &[0.1, 0.2, 0.3, 0.25]),
            series(&[("container", "sidecar")], &[0.01, 0.02, 0.01, 0.02]),
        ]),
    );
    prom.answer(
        "container:memory_usage",
        Ok(vec![
            series(&[("container", "app")], &[2.0e8, 2.2e8, 2.4e8]),
            series(&[("container", "sidecar")], &[3.0e7, 3.1e7, 3.0e7]),
        ]),
    );
    prom.answer(
        "container:cpu_requests",
        Ok(vec![
            series(&[("container", "app")], &[0.25]),
            series(&[("container", "sidecar")], &[0.05]),
        ]),
    );
    prom.answer(
        "container:cpu_limits",
        Ok(vec![series(&[("container", "app")], &[0.5])]),
    );
    prom.answer(
        "container:memory_limits",
        Ok(vec![series(&[("container", "app")], &[5.0e8])]),
    );
    prom.answer(
        "pod:network_rx",
        Ok(vec![series(&[("pod", "web-0")], &[100.0, 200.0])]),
    );
}

struct Rig {
    d: Detail,
    prom: Arc<Prom>,
    source: Arc<Source>,
    catalogue: Arc<FakeQueryCataloguePort>,
}

fn ready() -> HistoryAvailability {
    HistoryAvailability::Ready { provider: KUBE }
}

fn rig(cx: &mut TestAppContext, availability: HistoryAvailability) -> Rig {
    rig_with(cx, availability, [web_pod()])
}

fn rig_with(
    cx: &mut TestAppContext,
    availability: HistoryAvailability,
    objects: impl IntoIterator<Item = oxikube_domain::Resource>,
) -> Rig {
    let mut d = Detail::new(cx, objects);
    let prom = Arc::new(Prom::default());
    let source = Source::new(availability);
    let catalogue = Arc::new(FakeQueryCataloguePort::new());
    let service = MetricsHistoryService::new(source.clone(), prom.clone(), catalogue.clone())
        .with_state(d.f.state.clone());
    d.f.vcx.update(|_, cx| MetricsHistory::install(service, cx));
    Rig {
        d,
        prom,
        source,
        catalogue,
    }
}

impl Rig {
    fn open(&mut self, name: &str) -> gpui::Entity<DetailView> {
        self.d.open(&pod_ref(name))
    }

    /// Opens the Metrics tab by clicking it and lets the first answer land.
    fn show_metrics(&mut self, view: &gpui::Entity<DetailView>) {
        self.d.click("detail-tab-metrics");
        self.d.settle();
        assert_eq!(self.d.read(view, |v| v.tab()), DetailTab::Metrics);
    }

    fn state(&mut self, view: &gpui::Entity<DetailView>) -> MetricsState {
        self.d.read(view, |v| v.metrics_state())
    }

    fn advance(&mut self, by: Duration) {
        self.d.f.vcx.executor().advance_clock(by);
        self.d.settle();
    }
}

#[gpui::test]
fn without_a_prometheus_there_is_no_metrics_tab_and_no_chart_frame(cx: &mut TestAppContext) {
    let mut rig = rig(
        cx,
        HistoryAvailability::Unavailable {
            reason: "no Prometheus-compatible service was found".into(),
        },
    );
    let view = rig.open("web-0");
    assert!(!rig.d.shown("detail-tab-metrics"), "no tab to click");
    assert_eq!(
        rig.d.read(&view, |v| v.tabs()),
        [
            DetailTab::Overview,
            DetailTab::Yaml,
            DetailTab::Describe,
            DetailTab::Events
        ]
    );
    // Asking for it anyway (a command, a key) does nothing: no frame, no query.
    rig.d
        .update(&view, |v, cx| v.set_tab(DetailTab::Metrics, cx));
    assert_eq!(rig.d.read(&view, |v| v.tab()), DetailTab::Overview);
    assert!(!rig.d.shown("detail-metrics"));
    assert!(!rig.d.shown("metrics-chart-cpu"));
    assert_eq!(rig.prom.queries(), 0);
    // The reason is still readable, for the settings page and the palette.
    assert_eq!(
        rig.state(&view),
        MetricsState::NoEndpoint("no Prometheus-compatible service was found".into())
    );
}

#[gpui::test]
fn a_kind_without_graphs_never_has_the_tab(cx: &mut TestAppContext) {
    let configmap = oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "resourceVersion": "1"},
        "data": {"a": "b"}
    }))
    .expect("a config map");
    let mut rig = rig_with(cx, ready(), [configmap]);
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "settings",
    );
    let view = rig.d.open(&target);
    assert!(!rig.d.shown("detail-tab-metrics"));
    assert!(!rig.d.read(&view, |v| v.metrics_offered()));
}

#[gpui::test]
fn the_pod_panel_has_per_container_lines_and_request_and_limit_lines(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    assert_eq!(rig.prom.queries(), 0, "the Overview asks nothing");
    assert!(rig.d.shown("detail-tab-metrics"));

    rig.show_metrics(&view);
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert_eq!(
        rig.d.read(&view, |v| v.metrics_chart_ids()),
        ["cpu", "memory", "network"]
    );
    assert!(rig.d.shown("metrics-toolbar"));
    assert!(rig.d.shown("metrics-chart-cpu"));
    assert!(!rig.d.shown("metrics-loading"));

    let cpu = rig.d.read(&view, |v| v.metrics_chart("cpu")).unwrap();
    rig.d.f.vcx.update(|_, cx| {
        let chart = cpu.read(cx);
        assert_eq!(chart.state(), ChartState::Plot);
        let names: Vec<_> = chart.series().iter().map(|s| s.name.to_string()).collect();
        assert_eq!(names, ["app", "sidecar"], "one line per container");
        let lines: Vec<_> = chart
            .threshold_lines()
            .iter()
            .map(|t| (t.label.to_string(), t.value))
            .collect();
        assert_eq!(
            lines,
            [
                ("app requests".to_owned(), 0.25),
                ("sidecar requests".to_owned(), 0.05),
                ("app limits".to_owned(), 0.5)
            ]
        );
    });

    // One hour by default, the step policy of the range, queries for the detected stack.
    assert_eq!(rig.d.read(&view, |v| v.metrics_range()), RangePreset::Hour1);
    let range = rig.prom.calls.lock()[0].1;
    assert_eq!(
        range.end.as_second() - range.start.as_second(),
        3600,
        "the last hour"
    );
    assert!(
        rig.catalogue
            .recorded_calls()
            .iter()
            .all(|call| call.provider == KUBE)
    );
    let kinds: Vec<MetricKind> = rig
        .catalogue
        .recorded_calls()
        .iter()
        .map(|call| call.request.kind)
        .collect();
    for wanted in [
        MetricKind::CpuUsage,
        MetricKind::CpuRequests,
        MetricKind::CpuLimits,
        MetricKind::MemoryUsage,
        MetricKind::MemoryRequests,
        MetricKind::MemoryLimits,
        MetricKind::NetworkRx,
        MetricKind::NetworkTx,
    ] {
        assert!(kinds.contains(&wanted), "{wanted} was asked for");
    }
}

#[gpui::test]
fn changing_the_range_shows_loading_at_once_cancels_the_old_query_and_is_remembered(
    cx: &mut TestAppContext,
) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let after_first = rig.d.read(&view, |v| v.metrics_fetches());
    assert_eq!(after_first, 1);

    // A slow Prometheus: the 6h query is still running when 24h is chosen.
    rig.prom.gated.store(true, Ordering::SeqCst);
    rig.d.click("metrics-range-6h");
    assert_eq!(
        rig.d.read(&view, |v| v.metrics_range()),
        RangePreset::Hours6
    );
    assert_eq!(rig.state(&view), MetricsState::Loading);
    assert!(rig.d.shown("metrics-loading"), "loading in the same frame");
    assert!(
        !rig.d.shown("metrics-chart-cpu"),
        "no chart frame while loading"
    );
    let in_flight = rig.prom.in_flight.load(Ordering::SeqCst);
    assert!(in_flight > 0, "the 6h queries are running");

    rig.d.click("metrics-range-24h");
    assert_eq!(
        rig.d.read(&view, |v| v.metrics_range()),
        RangePreset::Hours24
    );
    // The 6h queries were dropped; only the 24h ones are in flight.
    assert_eq!(rig.prom.in_flight.load(Ordering::SeqCst), in_flight);
    assert_eq!(rig.d.read(&view, |v| v.metrics_fetches()), after_first + 2);
    let last = rig.prom.calls.lock().last().unwrap().1;
    assert_eq!(last.end.as_second() - last.start.as_second(), 24 * 3600);

    // Releasing answers only the 24h ones.
    rig.prom.gated.store(false, Ordering::SeqCst);
    rig.prom.release.notify_waiters();
    rig.d.settle();
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert_eq!(rig.prom.in_flight.load(Ordering::SeqCst), 0);

    // The choice outlives the view: another pod opens on 24h.
    let other = oxikube_testkit::pod()
        .namespace("shop")
        .name("web-1")
        .build();
    rig.d.f.ports().resources.insert(other);
    rig.d.settle();
    let second = rig.open("web-1");
    rig.show_metrics(&second);
    assert_eq!(
        rig.d.read(&second, |v| v.metrics_range()),
        RangePreset::Hours24
    );
}

#[gpui::test]
fn the_tab_refreshes_on_the_cadence_while_shown_and_not_otherwise(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let first = rig.prom.queries();
    assert!(first > 0);

    // Drawing never asks.
    for _ in 0..5 {
        rig.d.draw();
    }
    assert_eq!(rig.prom.queries(), first);

    // The default cadence is 30 s.
    rig.advance(Duration::from_secs(29));
    assert_eq!(rig.prom.queries(), first, "not yet");
    rig.advance(Duration::from_secs(2));
    assert_eq!(rig.prom.queries(), first * 2, "one refresh");
    assert_eq!(rig.state(&view), MetricsState::Ready, "no flash of loading");

    // Off the tab, nothing is asked however long it stays away.
    rig.d.click("detail-tab-overview");
    let before = rig.prom.queries();
    rig.advance(Duration::from_secs(300));
    assert_eq!(rig.prom.queries(), before);
    // Back on it, it asks again at once.
    rig.d.click("detail-tab-metrics");
    assert_eq!(rig.prom.queries(), before + first);

    // A hidden view (another item in the pane) does not ask either.
    rig.d.update(&view, |v, cx| v.set_shown(false, cx));
    let hidden = rig.prom.queries();
    rig.advance(Duration::from_secs(300));
    assert_eq!(rig.prom.queries(), hidden);
    rig.d.update(&view, |v, cx| v.set_shown(true, cx));
    assert!(rig.prom.queries() > hidden);
}

#[gpui::test]
fn closing_the_view_cancels_the_query_in_flight(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    rig.prom.gated.store(true, Ordering::SeqCst);
    let view = rig.open("web-0");
    rig.d.click("detail-tab-metrics");
    assert!(rig.prom.in_flight.load(Ordering::SeqCst) > 0);
    assert_eq!(rig.state(&view), MetricsState::Loading);
    rig.d.update(&view, |v, _| v.release());
    rig.d.settle();
    assert_eq!(rig.prom.in_flight.load(Ordering::SeqCst), 0);
}

#[gpui::test]
fn an_error_is_inline_with_the_reason_and_retry_asks_again(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    rig.prom.answer(
        "cpu_usage",
        Err(PromqlError::Unreachable("connection refused".into())),
    );
    rig.prom.answer(
        "memory_usage",
        Err(PromqlError::Unreachable("connection refused".into())),
    );
    rig.prom.answer(
        "network_rx",
        Err(PromqlError::Unreachable("connection refused".into())),
    );
    rig.prom.answer(
        "network_tx",
        Err(PromqlError::Unreachable("connection refused".into())),
    );
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let MetricsState::Error { message, retryable } = rig.state(&view) else {
        panic!("an error state");
    };
    assert!(message.starts_with("metrics unavailable: "), "{message}");
    assert!(message.contains("connection refused"), "{message}");
    assert!(retryable);
    assert!(rig.d.shown("metrics-error"));
    assert!(rig.d.shown("metrics-retry"));
    assert!(!rig.d.shown("metrics-chart-cpu"), "no empty chart frame");
    assert!(
        rig.d.shown("metrics-range-1h"),
        "the range is still choosable"
    );

    // Prometheus comes back; Retry loads.
    rig.prom.answers.lock().clear();
    two_containers(&rig.prom);
    rig.d.click("metrics-retry");
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert!(rig.d.shown("metrics-chart-cpu"));
}

#[gpui::test]
fn an_endpoint_with_no_series_for_the_object_says_so_instead_of_drawing_zeros(
    cx: &mut TestAppContext,
) {
    let mut rig = rig(cx, ready());
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    assert_eq!(rig.state(&view), MetricsState::Empty);
    assert!(rig.d.shown("metrics-empty"));
    assert!(!rig.d.shown("metrics-chart-cpu"), "no empty chart frame");
    assert!(!rig.d.shown("metrics-error"));
}

#[gpui::test]
fn the_tab_follows_the_availability_of_the_provider(cx: &mut TestAppContext) {
    let mut rig = rig(cx, HistoryAvailability::Detecting);
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    // Detection is running: the tab is offered and waits, asking nothing.
    assert!(rig.d.shown("detail-tab-metrics"));
    rig.show_metrics(&view);
    assert_eq!(rig.state(&view), MetricsState::Loading);
    assert!(rig.d.shown("metrics-loading"));
    assert_eq!(rig.prom.queries(), 0);

    // It finds one: the loop starts.
    rig.source.set(ready());
    rig.d.settle();
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert!(rig.prom.queries() > 0);

    // It goes away (the cluster is turned off in settings): the tab goes, the view returns to
    // the Overview and nothing is asked any more.
    rig.source.set(HistoryAvailability::Unavailable {
        reason: "turned off in this cluster's settings".into(),
    });
    rig.d.settle();
    assert_eq!(rig.d.read(&view, |v| v.tab()), DetailTab::Overview);
    assert!(!rig.d.shown("detail-tab-metrics"));
    let queries = rig.prom.queries();
    rig.advance(Duration::from_secs(300));
    assert_eq!(rig.prom.queries(), queries);
}

#[gpui::test]
fn a_workload_graphs_the_pods_it_selects_one_line_each(cx: &mut TestAppContext) {
    let deployment = oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "uid": "dep-uid", "resourceVersion": "10",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"replicas": 2, "selector": {"matchLabels": {"app": "web"}},
                 "template": {"metadata": {"labels": {"app": "web"}},
                              "spec": {"containers": [{"name": "app", "image": "shop/web:2"}]}}},
        "status": {"replicas": 2, "readyReplicas": 2}
    }))
    .expect("a deployment");
    let pods = ["web-a", "web-b"].map(|name| {
        oxikube_testkit::pod()
            .namespace("shop")
            .name(name)
            .label("app", "web")
            .build()
    });
    let stranger = oxikube_testkit::pod()
        .namespace("shop")
        .name("other-pod")
        .label("app", "other")
        .build();
    let mut rig = rig_with(
        cx,
        ready(),
        [deployment].into_iter().chain(pods).chain([stranger]),
    );
    rig.prom.answer(
        "pod:cpu_usage",
        Ok(vec![
            series(&[("pod", "web-a")], &[0.1, 0.2]),
            series(&[("pod", "web-b")], &[0.3, 0.1]),
        ]),
    );
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    let view = rig.d.open(&target);
    rig.d.click("detail-tab-metrics");
    rig.d.settle();
    assert_eq!(rig.state(&view), MetricsState::Ready);

    // The queries name exactly the pods the selector picks, sorted.
    let requested: Vec<_> = rig
        .catalogue
        .recorded_calls()
        .iter()
        .filter(|c| c.request.kind == MetricKind::CpuUsage)
        .map(|c| c.request.target.clone())
        .collect();
    assert_eq!(
        requested,
        [oxikube_domain::MetricTarget::Pod {
            namespace: "shop".into(),
            pods: oxikube_domain::PodSelector::Any(vec!["web-a".into(), "web-b".into()]),
        }]
    );
    let cpu = rig.d.read(&view, |v| v.metrics_chart("cpu")).unwrap();
    rig.d.f.vcx.update(|_, cx| {
        let names: Vec<_> = cpu
            .read(cx)
            .series()
            .iter()
            .map(|s| s.name.to_string())
            .collect();
        assert_eq!(names, ["web-a", "web-b"]);
    });
}

#[gpui::test]
fn nothing_but_the_loop_fetches(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let fetches = rig.d.read(&view, |v| v.metrics_fetches());
    let queries = rig.prom.queries();
    // Redraws, hovers, tab-strip clicks on the same tab and resizes cost no query.
    for _ in 0..20 {
        rig.d.draw();
    }
    rig.d.click("detail-tab-metrics");
    rig.d.click("metrics-range-1h");
    assert_eq!(rig.d.read(&view, |v| v.metrics_fetches()), fetches);
    assert_eq!(rig.prom.queries(), queries);
}

fn send_view_metrics(rig: &mut Rig, target: &ResourceRef) {
    let views = rig.d.f.views.clone();
    let target = target.clone();
    rig.d.f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            let request = crate::ViewRequest::ViewMetrics(target);
            views.apply_in(request, window, cx);
        })
    });
    rig.d.settle();
}

fn toasts(rig: &mut Rig) -> Vec<String> {
    let tab = rig
        .d
        .f
        .vcx
        .update(|_, cx| rig.d.f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    rig.d.f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

#[gpui::test]
fn the_view_metrics_command_opens_the_detail_on_the_metrics_tab(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    send_view_metrics(&mut rig, &pod_ref("web-0"));
    let view = rig.d.drawer_view().expect("the drawer opened");
    assert_eq!(rig.d.read(&view, |v| v.tab()), DetailTab::Metrics);
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert!(toasts(&mut rig).is_empty());
    // The command is the one the palette and an agent run.
    assert_eq!(
        crate::ResourceCommandSink::request_for(
            &oxikube_domain::command::Command::ResourceViewMetrics {
                target: pod_ref("web-0")
            }
        ),
        Some(crate::ViewRequest::ViewMetrics(pod_ref("web-0")))
    );
}

#[gpui::test]
fn the_view_metrics_command_says_why_when_there_is_no_prometheus(cx: &mut TestAppContext) {
    let mut rig = rig(
        cx,
        HistoryAvailability::Unavailable {
            reason: "no Prometheus-compatible service was found".into(),
        },
    );
    send_view_metrics(&mut rig, &pod_ref("web-0"));
    let view = rig.d.drawer_view().expect("the drawer opened");
    assert_eq!(rig.d.read(&view, |v| v.tab()), DetailTab::Overview);
    assert_eq!(
        toasts(&mut rig),
        ["metrics unavailable: no Prometheus-compatible service was found"]
    );
    assert_eq!(rig.prom.queries(), 0);
}

/// A workload `kind` named `name` in `shop` that selects `app=web`.
fn controller(kind: &str, name: &str) -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "apps/v1", "kind": kind,
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"),
                     "resourceVersion": "10", "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"selector": {"matchLabels": {"app": "web"}},
                 "template": {"metadata": {"labels": {"app": "web"}},
                              "spec": {"containers": [{"name": "app", "image": "shop/web:2"}]}}},
        "status": {}
    }))
    .expect("a controller")
}

fn web_pods() -> [oxikube_domain::Resource; 2] {
    ["web-a", "web-b"].map(|name| {
        oxikube_testkit::pod()
            .namespace("shop")
            .name(name)
            .label("app", "web")
            .build()
    })
}

fn controller_ref(kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("apps", "v1", kind), "shop", name)
}

#[gpui::test]
fn a_daemon_set_graphs_the_pods_it_selects(cx: &mut TestAppContext) {
    let mut rig = rig_with(
        cx,
        ready(),
        [controller("DaemonSet", "proxy")]
            .into_iter()
            .chain(web_pods()),
    );
    rig.prom.answer(
        "pod:cpu_usage",
        Ok(vec![
            series(&[("pod", "web-a")], &[0.1, 0.2]),
            series(&[("pod", "web-b")], &[0.3, 0.1]),
        ]),
    );
    let view = rig.d.open(&controller_ref("DaemonSet", "proxy"));
    rig.d.click("detail-tab-metrics");
    rig.d.settle();
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert!(rig.d.shown("metrics-chart-cpu"));
    let pods: Vec<_> = rig
        .catalogue
        .recorded_calls()
        .iter()
        .filter(|c| c.request.kind == MetricKind::CpuUsage)
        .map(|c| c.request.target.clone())
        .collect();
    assert_eq!(
        pods,
        [oxikube_domain::MetricTarget::Pod {
            namespace: "shop".into(),
            pods: oxikube_domain::PodSelector::Any(vec!["web-a".into(), "web-b".into()]),
        }]
    );
}

#[gpui::test]
fn a_refused_pods_feed_says_why_and_retry_subscribes_again(cx: &mut TestAppContext) {
    let mut rig = rig_with(
        cx,
        ready(),
        [controller("Deployment", "web")]
            .into_iter()
            .chain(web_pods()),
    );
    rig.prom.answer(
        "pod:cpu_usage",
        Ok(vec![series(&[("pod", "web-a")], &[0.1, 0.2])]),
    );
    let view = rig.d.open(&controller_ref("Deployment", "web"));
    // The user may read the Deployment but not list its pods: the next watch (the pods) is refused.
    rig.d
        .f
        .ports()
        .resources
        .script()
        .watch
        .push_err(oxikube_domain::OxiError::forbidden(
            "pods is forbidden: User cannot list resource \"pods\"",
        ));
    rig.d.click("detail-tab-metrics");
    rig.d.settle();
    let MetricsState::Error { message, retryable } = rig.state(&view) else {
        panic!("the skeleton must not stay: {:?}", rig.state(&view));
    };
    assert!(message.starts_with("metrics unavailable: "), "{message}");
    assert!(message.contains("forbidden"), "{message}");
    assert!(retryable);
    assert!(rig.d.shown("metrics-error"));
    assert_eq!(rig.prom.queries(), 0, "nothing can be asked without pods");

    // Access is granted; Retry subscribes to the pods again and loads.
    rig.d.click("metrics-retry");
    rig.d.settle();
    assert_eq!(rig.state(&view), MetricsState::Ready);
    assert!(rig.d.shown("metrics-chart-cpu"));
}

#[gpui::test]
fn the_chart_names_follow_the_range(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let label = |rig: &mut Rig| {
        let cpu = rig.d.read(&view, |v| v.metrics_chart("cpu")).unwrap();
        rig.d
            .f
            .vcx
            .update(|_, cx| cpu.read(cx).label_text().map(|l| l.to_string()))
    };
    let first = label(&mut rig).expect("a label");
    assert!(first.starts_with("CPU, last "), "{first}");
    let other = if rig.d.read(&view, |v| v.metrics_range()) == RangePreset::Hours24 {
        RangePreset::Hour1
    } else {
        RangePreset::Hours24
    };
    rig.d.update(&view, |v, cx| v.set_metrics_range(other, cx));
    rig.d.settle();
    let second = label(&mut rig).expect("a label");
    assert_ne!(first, second);
    assert!(second.ends_with(other.label()), "{second}");
}

#[gpui::test]
fn a_refresh_with_the_same_cards_keeps_the_scroll_position(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    let view = rig.open("web-0");
    // A drawer too short for the three cards, so the list scrolls.
    rig.d
        .f
        .vcx
        .simulate_resize(gpui::size(gpui::px(1920.), gpui::px(500.)));
    rig.show_metrics(&view);
    rig.d.draw();
    let list = rig.d.read(&view, |v| v.metrics.list.clone());
    list.scroll_to(gpui::ListOffset {
        item_ix: 1,
        offset_in_item: gpui::px(12.),
    });
    rig.d.draw();
    assert_eq!(list.logical_scroll_top().item_ix, 1, "the list scrolls");
    let queries = rig.prom.queries();
    rig.advance(Duration::from_secs(31));
    rig.d.draw();
    assert!(rig.prom.queries() > queries, "the refresh ran");
    assert_eq!(list.logical_scroll_top().item_ix, 1);
    assert_eq!(list.logical_scroll_top().offset_in_item, gpui::px(12.));
}

#[gpui::test]
fn a_fetch_that_dies_is_an_error_with_retry_not_an_empty_answer(cx: &mut TestAppContext) {
    let mut rig = rig(cx, ready());
    two_containers(&rig.prom);
    rig.prom.panics.store(true, Ordering::SeqCst);
    let view = rig.open("web-0");
    rig.show_metrics(&view);
    let MetricsState::Error { message, retryable } = rig.state(&view) else {
        panic!("a dead fetch is an error: {:?}", rig.state(&view));
    };
    assert!(message.starts_with("metrics unavailable: "), "{message}");
    assert!(retryable);
    assert!(rig.d.shown("metrics-retry"));
    assert!(!rig.d.shown("metrics-empty"));

    rig.prom.panics.store(false, Ordering::SeqCst);
    rig.d.click("metrics-retry");
    assert_eq!(rig.state(&view), MetricsState::Ready);
}
