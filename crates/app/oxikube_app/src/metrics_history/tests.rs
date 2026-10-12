//! The history service over a scripted `PromqlPort`, the testkit's placeholder catalogue and a
//! scripted availability. No cluster, no HTTP.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::{
    MetricKind, MetricUnit, MetricsProviderKind, OxiError, OxiResult, RangePreset,
};
use oxikube_ports::{PromqlError, PromqlPort, PromqlSeries, PromqlValue, TimeRange};
use oxikube_testkit::{FakeQueryCataloguePort, FakeStatePort};
use parking_lot::Mutex;

use super::{
    ChartDef, HistoryAvailability, HistorySource, HistorySubject, MAX_WORKLOAD_PODS,
    MetricsHistoryService, ReferenceKind, SubjectKind, charts_for,
};

fn cluster() -> ClusterId {
    ClusterId::new("/kube/config", &ContextName::new("test"))
}

fn end() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

struct Source(Mutex<HistoryAvailability>);

impl Source {
    fn ready() -> Arc<Self> {
        Arc::new(Self(Mutex::new(HistoryAvailability::Ready {
            provider: MetricsProviderKind::KubePrometheus,
        })))
    }
}

impl HistorySource for Source {
    fn availability(&self, _: &ClusterId) -> HistoryAvailability {
        self.0.lock().clone()
    }

    fn changes(&self, _: &ClusterId) -> BoxStream<'static, ()> {
        futures::stream::empty().boxed()
    }
}

fn series(labels: &[(&str, &str)], values: &[f64]) -> PromqlSeries {
    PromqlSeries {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        points: values
            .iter()
            .enumerate()
            .map(|(i, v)| PromqlValue {
                ts: end() - Duration::from_secs(60 * (values.len() - i) as u64),
                value: *v,
            })
            .collect(),
    }
}

/// Answers by the placeholder query text the fake catalogue builds
/// (`fake:<provider>:<scope>:<kind>:<window>s`).
#[derive(Default)]
struct Prom {
    answers: Mutex<Vec<(String, Result<Vec<PromqlSeries>, PromqlError>)>>,
    calls: Mutex<Vec<(String, TimeRange)>>,
    hang: AtomicBool,
    in_flight: AtomicUsize,
    dropped: AtomicUsize,
}

impl Prom {
    fn answer(&self, needle: &str, result: Result<Vec<PromqlSeries>, PromqlError>) {
        self.answers.lock().push((needle.to_owned(), result));
    }
}

struct DropCount<'a>(&'a Prom);

impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
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
        if self.hang.load(Ordering::SeqCst) {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            let _guard = DropCount(self);
            futures::future::pending::<()>().await;
        }
        let found = self
            .answers
            .lock()
            .iter()
            .find(|(needle, _)| query.contains(needle.as_str()))
            .map(|(_, result)| result.clone().map_err(OxiError::from));
        found.unwrap_or_else(|| Ok(Vec::new()))
    }
}

fn service(prom: &Arc<Prom>) -> (MetricsHistoryService, Arc<FakeQueryCataloguePort>) {
    let catalogue = Arc::new(FakeQueryCataloguePort::new());
    let service = MetricsHistoryService::new(Source::ready(), prom.clone(), catalogue.clone());
    (service, catalogue)
}

fn pod(namespace: &str, name: &str) -> HistorySubject {
    HistorySubject::new(SubjectKind::Pod, Some(namespace), name, []).unwrap()
}

fn kinds_of(chart: &ChartDef) -> Vec<MetricKind> {
    chart.measures.iter().map(|m| m.request.kind).collect()
}

#[test]
fn the_six_kinds_have_graphs_and_the_others_do_not() {
    let kind = |group: &str, kind: &str| SubjectKind::of(&Gvk::new(group, "v1", kind));
    assert_eq!(kind("", "Pod"), Some(SubjectKind::Pod));
    assert_eq!(kind("", "Node"), Some(SubjectKind::Node));
    assert_eq!(kind("", "Namespace"), Some(SubjectKind::Namespace));
    assert_eq!(kind("", "PersistentVolumeClaim"), Some(SubjectKind::Pvc));
    assert_eq!(kind("apps", "Deployment"), Some(SubjectKind::Workload));
    assert_eq!(kind("apps", "StatefulSet"), Some(SubjectKind::Workload));
    assert_eq!(
        kind("networking.k8s.io", "Ingress"),
        Some(SubjectKind::Ingress)
    );
    assert_eq!(kind("", "ConfigMap"), None);
    assert_eq!(kind("batch", "Job"), None);
    // A custom resource that happens to be called Pod is not a Pod.
    assert_eq!(kind("example.com", "Pod"), None);
}

#[test]
fn every_chart_of_every_kind_is_a_query_that_exists() {
    let subjects = [
        pod("shop", "web-0"),
        HistorySubject::new(SubjectKind::Node, None, "n1", []).unwrap(),
        HistorySubject::new(SubjectKind::Namespace, None, "shop", []).unwrap(),
        HistorySubject::new(SubjectKind::Pvc, Some("shop"), "data", []).unwrap(),
        HistorySubject::new(SubjectKind::Ingress, Some("shop"), "web", []).unwrap(),
        HistorySubject::new(
            SubjectKind::Workload,
            Some("shop"),
            "web",
            ["web-0".to_owned()],
        )
        .unwrap(),
    ];
    for subject in subjects {
        let charts = charts_for(&subject);
        assert!(!charts.is_empty(), "{subject:?}");
        for chart in &charts {
            for request in chart
                .measures
                .iter()
                .map(|m| &m.request)
                .chain(chart.references.iter().map(|r| &r.request))
            {
                request.validate().unwrap_or_else(|error| {
                    panic!("{subject:?} {}: {error}", chart.id);
                });
                assert_eq!(request.kind.unit(), chart.unit, "{}", chart.id);
            }
            for reference in &chart.references {
                assert!(reference.request.kind.is_reference());
            }
        }
    }
}

#[test]
fn a_pod_graphs_cpu_and_memory_per_container_with_requests_and_limits() {
    let charts = charts_for(&pod("shop", "web-0"));
    let ids: Vec<_> = charts.iter().map(|c| c.id).collect();
    assert_eq!(ids, ["cpu", "memory", "network"]);
    let cpu = &charts[0];
    assert_eq!(
        cpu.measures[0].request.target.series_label(),
        Some("container")
    );
    let refs: Vec<_> = cpu.references.iter().map(|r| r.request.kind).collect();
    assert_eq!(refs, [MetricKind::CpuRequests, MetricKind::CpuLimits]);
    let memory = &charts[1];
    let refs: Vec<_> = memory.references.iter().map(|r| r.request.kind).collect();
    assert_eq!(refs, [MetricKind::MemoryRequests, MetricKind::MemoryLimits]);
    assert_eq!(
        kinds_of(&charts[2]),
        [MetricKind::NetworkRx, MetricKind::NetworkTx]
    );
}

#[test]
fn each_kind_asks_the_right_query_set() {
    let ids = |subject: &HistorySubject| -> Vec<&'static str> {
        charts_for(subject).iter().map(|c| c.id).collect()
    };
    let node = HistorySubject::new(SubjectKind::Node, None, "n1", []).unwrap();
    assert_eq!(ids(&node), ["cpu", "memory", "network", "disk"]);
    let ns = HistorySubject::new(SubjectKind::Namespace, None, "shop", []).unwrap();
    assert_eq!(ids(&ns), ["cpu", "memory", "network", "pods"]);
    let pvc = HistorySubject::new(SubjectKind::Pvc, Some("shop"), "data", []).unwrap();
    let disk = &charts_for(&pvc)[0];
    assert_eq!(kinds_of(disk), [MetricKind::DiskUsage]);
    assert_eq!(disk.references[0].request.kind, MetricKind::DiskCapacity);
    let ingress = HistorySubject::new(SubjectKind::Ingress, Some("shop"), "web", []).unwrap();
    assert_eq!(
        kinds_of(&charts_for(&ingress)[0]),
        [MetricKind::NetworkRx, MetricKind::NetworkTx]
    );
}

#[test]
fn a_workload_is_sorted_capped_and_counts_what_it_left_out() {
    let names = (0..30).rev().map(|i| format!("web-{i:02}"));
    let subject = HistorySubject::new(SubjectKind::Workload, Some("shop"), "web", names).unwrap();
    let HistorySubject::Workload { pods, total, .. } = &subject else {
        panic!("a workload");
    };
    assert_eq!(pods.len(), MAX_WORKLOAD_PODS);
    assert_eq!(pods[0], "web-00");
    assert_eq!(*total, 30);
    assert_eq!(subject.pods_left_out(), 10);
    // No pods: nothing to ask, and the view says so.
    let none = HistorySubject::new(SubjectKind::Workload, Some("shop"), "web", []).unwrap();
    assert!(none.is_empty());
    assert!(charts_for(&none).is_empty());
    // A namespaced kind needs its namespace.
    assert!(HistorySubject::new(SubjectKind::Pod, None, "web-0", []).is_none());
}

#[tokio::test]
async fn per_container_series_and_threshold_lines_are_built_from_the_fixture() {
    let prom = Arc::new(Prom::default());
    prom.answer(
        "container:cpu_usage",
        Ok(vec![
            series(&[("container", "app")], &[0.1, 0.2, 0.3]),
            series(&[("container", "sidecar")], &[0.01, 0.02, 0.01]),
        ]),
    );
    prom.answer(
        "container:cpu_requests",
        Ok(vec![
            series(&[("container", "app")], &[0.5, 0.5]),
            series(&[("container", "sidecar")], &[0.1, 0.1]),
        ]),
    );
    // One container has a limit, the other none: no series for it.
    prom.answer(
        "container:cpu_limits",
        Ok(vec![series(&[("container", "app")], &[1.0, 1.0])]),
    );
    let (service, catalogue) = service(&prom);
    let charts = charts_for(&pod("shop", "web-0"));
    let results = service
        .fetch(&cluster(), &charts[..1], RangePreset::Hour1, end())
        .await;
    let cpu = results[0].as_ref().unwrap();

    let names: Vec<_> = cpu.series.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["app", "sidecar"], "one line per container");
    assert_eq!(cpu.series[0].samples.len(), 3);
    assert_eq!(cpu.unit, MetricUnit::Cores);
    let lines: Vec<_> = cpu
        .references
        .iter()
        .map(|r| (r.label.as_str(), r.value, r.kind))
        .collect();
    assert_eq!(
        lines,
        [
            ("app requests", 0.5, ReferenceKind::Soft),
            ("sidecar requests", 0.1, ReferenceKind::Soft),
            ("app limits", 1.0, ReferenceKind::Hard),
        ],
        "differing requests are named by container; a limit only one container has keeps its name"
    );
    assert!(!cpu.is_empty());

    // The provider found by detection picked the queries, and the rate window is the policy's.
    let calls = catalogue.recorded_calls();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|c| c.provider == MetricsProviderKind::KubePrometheus
                && c.rate_window == Duration::from_secs(120))
    );
    // The range is the preset's: one hour ending at `end`, 15 s step.
    let prom_calls = prom.calls.lock();
    let range = prom_calls[0].1;
    assert_eq!(range.end, end());
    assert_eq!(range.start, end() - Duration::from_secs(3600));
    assert_eq!(range.step, Duration::from_secs(15));
}

#[tokio::test]
async fn a_workloads_equal_requests_become_one_line() {
    let prom = Arc::new(Prom::default());
    prom.answer(
        "pod:memory_usage",
        Ok(vec![
            series(&[("pod", "web-0")], &[1e8, 2e8]),
            series(&[("pod", "web-1")], &[1e8, 3e8]),
        ]),
    );
    prom.answer(
        "memory_requests",
        Ok(vec![
            series(&[("pod", "web-0")], &[2.5e8]),
            series(&[("pod", "web-1")], &[2.5e8]),
        ]),
    );
    let (service, _) = service(&prom);
    let subject = HistorySubject::new(
        SubjectKind::Workload,
        Some("shop"),
        "web",
        ["web-1".to_owned(), "web-0".to_owned()],
    )
    .unwrap();
    let charts = charts_for(&subject);
    let results = service
        .fetch(&cluster(), &charts[1..2], RangePreset::Hours6, end())
        .await;
    let memory = results[0].as_ref().unwrap();
    assert_eq!(memory.series.len(), 2);
    assert_eq!(memory.references.len(), 1);
    assert_eq!(memory.references[0].label, "requests");
    assert_eq!(memory.references[0].value, 2.5e8);
    assert_eq!(prom.calls.lock()[0].1.step, Duration::from_secs(120));
}

#[tokio::test]
async fn several_measures_share_a_chart_and_name_their_unlabelled_series() {
    let prom = Arc::new(Prom::default());
    prom.answer("network_rx", Ok(vec![series(&[], &[10.0, 20.0])]));
    prom.answer("network_tx", Ok(vec![series(&[], &[1.0, 2.0])]));
    let (service, _) = service(&prom);
    let node = HistorySubject::new(SubjectKind::Node, None, "n1", []).unwrap();
    let charts = charts_for(&node);
    let network = charts.iter().position(|c| c.id == "network").unwrap();
    let results = service
        .fetch(
            &cluster(),
            &charts[network..=network],
            RangePreset::Minutes15,
            end(),
        )
        .await;
    let names: Vec<_> = results[0]
        .as_ref()
        .unwrap()
        .series
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(names, ["received", "sent"]);
}

#[tokio::test]
async fn an_endpoint_with_no_series_is_empty_not_a_chart_of_zeros() {
    let prom = Arc::new(Prom::default());
    let (service, _) = service(&prom);
    let charts = charts_for(&pod("shop", "web-0"));
    let results = service
        .fetch(&cluster(), &charts, RangePreset::Hour1, end())
        .await;
    for result in results {
        let data = result.unwrap();
        assert!(data.is_empty());
        assert!(data.series.is_empty() && data.references.is_empty());
    }
}

#[tokio::test]
async fn a_failed_measure_fails_the_chart_with_the_reason_and_a_failed_reference_does_not() {
    let prom = Arc::new(Prom::default());
    prom.answer(
        "container:cpu_usage",
        Err(PromqlError::Unreachable("connection refused".into())),
    );
    prom.answer(
        "container:memory_usage",
        Ok(vec![series(&[("container", "app")], &[1.0])]),
    );
    prom.answer(
        "memory_limits",
        Err(PromqlError::Timeout(Duration::from_secs(3))),
    );
    let (service, _) = service(&prom);
    let charts = charts_for(&pod("shop", "web-0"));
    let results = service
        .fetch(&cluster(), &charts[..2], RangePreset::Hour1, end())
        .await;
    let error = results[0].as_ref().unwrap_err();
    assert!(error.reason.contains("connection refused"), "{error:?}");
    assert!(error.retryable);
    assert!(error.text().starts_with("metrics unavailable: "));
    let memory = results[1].as_ref().expect("the memory chart still draws");
    assert_eq!(memory.series.len(), 1);
    assert!(memory.references.is_empty());
}

#[tokio::test]
async fn forbidden_is_a_reason_that_cannot_be_retried() {
    let prom = Arc::new(Prom::default());
    prom.answer(
        "cpu_usage",
        Err(PromqlError::Forbidden("403 from the service proxy".into())),
    );
    let (service, _) = service(&prom);
    let charts = charts_for(&pod("shop", "web-0"));
    let results = service
        .fetch(&cluster(), &charts[..1], RangePreset::Hour1, end())
        .await;
    let error = results[0].as_ref().unwrap_err();
    assert!(error.reason.contains("not allowed"), "{error:?}");
    assert!(!error.retryable);
}

#[tokio::test]
async fn no_endpoint_fails_every_chart_without_asking_prometheus() {
    let prom = Arc::new(Prom::default());
    let source = Arc::new(Source(Mutex::new(HistoryAvailability::Unavailable {
        reason: "no Prometheus-compatible service was found".into(),
    })));
    let service = MetricsHistoryService::new(
        source.clone(),
        prom.clone(),
        Arc::new(FakeQueryCataloguePort::new()),
    );
    assert!(!service.availability(&cluster()).offers_tab());
    let charts = charts_for(&pod("shop", "web-0"));
    let results = service
        .fetch(&cluster(), &charts, RangePreset::Hour1, end())
        .await;
    assert!(results.iter().all(|r| r.is_err()));
    assert_eq!(
        results[0].as_ref().unwrap_err().text(),
        "metrics unavailable: no Prometheus-compatible service was found"
    );
    assert!(prom.calls.lock().is_empty());

    *source.0.lock() = HistoryAvailability::Detecting;
    assert!(service.availability(&cluster()).offers_tab());
    let results = service
        .fetch(&cluster(), &charts, RangePreset::Hour1, end())
        .await;
    assert!(results[0].as_ref().unwrap_err().retryable);
    assert!(prom.calls.lock().is_empty());
}

#[tokio::test]
async fn dropping_a_fetch_cancels_its_queries() {
    let prom = Arc::new(Prom::default());
    prom.hang.store(true, Ordering::SeqCst);
    let (service, _) = service(&prom);
    let charts = charts_for(&pod("shop", "web-0"));
    let id = cluster();
    let mut fetch = Box::pin(service.fetch(&id, &charts, RangePreset::Hour1, end()));
    // Poll once so the queries start, then drop the future as a range change does.
    assert!(futures::poll!(fetch.as_mut()).is_pending());
    assert!(prom.in_flight.load(Ordering::SeqCst) > 0);
    drop(fetch);
    assert_eq!(
        prom.in_flight.load(Ordering::SeqCst),
        0,
        "no query outlives its fetch"
    );
    assert!(prom.dropped.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn the_chosen_range_is_remembered_and_a_bad_row_reads_as_the_default() {
    let prom = Arc::new(Prom::default());
    let state = Arc::new(FakeStatePort::new());
    let (service, _) = service(&prom);
    let service = service.with_state(state.clone());
    assert_eq!(service.saved_range().await, RangePreset::Hour1);
    service.save_range(RangePreset::Hours6).await;
    assert_eq!(service.saved_range().await, RangePreset::Hours6);
    service.save_range(RangePreset::Minutes15).await;
    assert_eq!(service.saved_range().await, RangePreset::Minutes15);

    use oxikube_ports::{StateKey, StatePort as _};
    state
        .kv_set(
            &StateKey::new("metrics.history_range").unwrap(),
            serde_json::json!("3d"),
        )
        .await
        .unwrap();
    assert_eq!(service.saved_range().await, RangePreset::Hour1);
    // Without a state port the range is the default and saving does nothing.
    let (bare, _) = self::service(&prom);
    bare.save_range(RangePreset::Hours24).await;
    assert_eq!(bare.saved_range().await, RangePreset::Hour1);
}

#[test]
fn availability_reads_the_provider_status() {
    use crate::metrics_provider::{ProviderState, ProviderStatus};
    use oxikube_ports::{DetectionState, MetricsMode};
    let off = ProviderStatus::new(MetricsMode::Off, ProviderState::Disabled);
    let HistoryAvailability::Unavailable { reason } = HistoryAvailability::of(&off) else {
        panic!("disabled is unavailable");
    };
    assert!(reason.contains("turned off"), "{reason}");
    let detecting = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Detecting),
    );
    assert_eq!(
        HistoryAvailability::of(&detecting),
        HistoryAvailability::Detecting
    );
    let missing = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::NotFound),
    );
    assert!(!HistoryAvailability::of(&missing).offers_tab());
    let _unused: Option<OxiError> = None;
}
