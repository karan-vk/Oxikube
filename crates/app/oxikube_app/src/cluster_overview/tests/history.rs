//! The history queries: four range queries built by the stack's catalogue for the cluster scope,
//! a failed requests line left out, a failed usage query reported, and availability read from the
//! provider status.

use std::sync::Arc;

use futures::executor::block_on;
use jiff::Timestamp;
use oxikube_domain::metric_query::MetricKind;
use oxikube_domain::{MetricsProviderKind, OxiError};
use oxikube_ports::{
    DetectedProvider, DetectionState, MetricsMode, PromqlSeries, PromqlValue, ProviderTarget,
};
use oxikube_testkit::{FakePromqlPort, FakeQueryCataloguePort};

use super::cluster;
use crate::ProviderStatus;
use crate::cluster_overview::{HISTORY_SPAN, HistoryAvailability, fetch_cluster_history};
use crate::metrics_provider::ProviderState;

fn series(values: &[f64]) -> Vec<PromqlSeries> {
    vec![PromqlSeries {
        labels: Default::default(),
        points: values
            .iter()
            .enumerate()
            .map(|(i, v)| PromqlValue {
                ts: Timestamp::from_second(1_700_000_000 + 60 * i as i64).expect("a time"),
                value: *v,
            })
            .collect(),
    }]
}

fn end() -> Timestamp {
    Timestamp::from_second(1_700_003_600).expect("a time")
}

#[test]
fn the_cluster_graphs_are_four_range_queries_from_the_catalogue() {
    let promql = Arc::new(FakePromqlPort::new());
    promql
        .script()
        .query_range
        .push_ok(series(&[1.0, 2.0]))
        .push_ok(series(&[3.0]))
        .push_ok(series(&[1e9]))
        .push_ok(series(&[2e9]));
    let catalogue = Arc::new(FakeQueryCataloguePort::new());
    let history = block_on(fetch_cluster_history(
        promql.clone(),
        catalogue.clone(),
        cluster(),
        MetricsProviderKind::KubePrometheus,
        end(),
    ))
    .expect("history");
    let kinds = |lines: &[crate::cluster_overview::HistoryLine]| {
        lines.iter().map(|l| l.kind).collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(&history.cpu),
        [MetricKind::CpuUsage, MetricKind::CpuRequests]
    );
    assert_eq!(
        kinds(&history.memory),
        [MetricKind::MemoryUsage, MetricKind::MemoryRequests]
    );
    assert_eq!(history.cpu[0].points.len(), 2);
    assert_eq!(history.end, end());
    assert_eq!(
        history.end.as_second() - history.start.as_second(),
        HISTORY_SPAN.as_secs() as i64
    );
    let calls = catalogue.recorded_calls();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|c| c.provider == MetricsProviderKind::KubePrometheus)
    );
    assert!(
        calls
            .iter()
            .all(|c| c.request.scope() == oxikube_domain::metric_query::MetricScope::Cluster)
    );
    assert_eq!(promql.recorded_calls().len(), 4);
}

#[test]
fn a_failed_requests_line_is_left_out_and_a_failed_usage_query_is_the_reason() {
    let promql = Arc::new(FakePromqlPort::new());
    promql
        .script()
        .query_range
        .push_ok(series(&[1.0]))
        .push_err(OxiError::network("reset"))
        .push_ok(series(&[1e9]))
        .push_ok(series(&[2e9]));
    let history = block_on(fetch_cluster_history(
        promql,
        Arc::new(FakeQueryCataloguePort::new()),
        cluster(),
        MetricsProviderKind::Lens,
        end(),
    ))
    .expect("history");
    assert_eq!(history.cpu.len(), 1, "requests left out");
    assert_eq!(history.memory.len(), 2);

    let promql = Arc::new(FakePromqlPort::new());
    promql
        .script()
        .query_range
        .push_err(OxiError::timeout("prometheus did not answer"))
        .push_ok(series(&[1.0]))
        .push_ok(series(&[1.0]))
        .push_ok(series(&[1.0]));
    let failed = block_on(fetch_cluster_history(
        promql,
        Arc::new(FakeQueryCataloguePort::new()),
        cluster(),
        MetricsProviderKind::Lens,
        end(),
    ));
    assert!(failed.is_err_and(|reason| reason.contains("did not answer")));
}

#[test]
fn availability_reads_the_provider_status() {
    let found = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Found(DetectedProvider {
            kind: MetricsProviderKind::VictoriaMetrics,
            target: ProviderTarget::Url("https://vm.example".into()),
            path_prefix: None,
        })),
    );
    assert_eq!(
        HistoryAvailability::of(&found),
        HistoryAvailability::Ready {
            provider: MetricsProviderKind::VictoriaMetrics
        }
    );
    let detecting = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Detecting),
    );
    assert_eq!(
        HistoryAvailability::of(&detecting),
        HistoryAvailability::Detecting
    );
    let off = ProviderStatus::new(MetricsMode::Off, ProviderState::Disabled);
    assert!(matches!(
        HistoryAvailability::of(&off),
        HistoryAvailability::Unavailable { reason } if reason.contains("turned off")
    ));
}
