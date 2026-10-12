//! Reading usage from the metrics port: a node's sample, the sum over a namespace's pods, and
//! every way metrics can be absent.

use jiff::Timestamp;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason, Reading};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{FakeMetricsPort, MetricsCall};

use super::{cluster, q};
use crate::allocation::{UsageState, namespace_usage, node_usage};

fn now() -> Timestamp {
    "2026-10-12T10:00:00Z".parse().unwrap()
}

fn node_sample(name: &str, cpu_nano: u64, memory: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        now(),
        None,
        cpu_nano,
        memory,
    )
}

fn pod_sample(ns: &str, name: &str, cpu_nano: u64, memory: u64) -> MetricsSample {
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), ns, name);
    MetricsSample::new(MetricsSubject::Pod { pod }, now(), None, cpu_nano, memory)
}

fn ready(state: UsageState) -> crate::allocation::MeasuredUsage {
    match state {
        UsageState::Ready(measured) => measured,
        other => panic!("expected a reading, got {other:?}"),
    }
}

fn text(state: &UsageState) -> &str {
    match state {
        UsageState::Unavailable(text) => text,
        other => panic!("expected an absence, got {other:?}"),
    }
}

#[tokio::test]
async fn a_node_reads_its_own_sample() {
    let port = FakeMetricsPort::new();
    port.script()
        .node_metrics
        .push_ok(MetricsOutcome::Available(vec![
            node_sample("other", 9_000_000_000, 1 << 30),
            node_sample("n1", 1_200_000_000, 3 << 30),
        ]));
    let usage = ready(node_usage(&port, &cluster(), "n1").await);
    assert_eq!(usage.cpu, Ok(q("1200m")));
    assert_eq!(usage.memory, Ok(q("3Gi")));
    assert_eq!(port.recorded_calls(), [MetricsCall::NodeMetrics(cluster())]);
}

#[tokio::test]
async fn a_node_without_a_sample_has_no_reading_yet() {
    let port = FakeMetricsPort::new();
    port.script()
        .node_metrics
        .push_ok(MetricsOutcome::Available(vec![node_sample("other", 1, 1)]));
    let state = node_usage(&port, &cluster(), "n1").await;
    assert_eq!(
        text(&state),
        "metrics unavailable: no reading yet (metrics-server has not scraped it)"
    );
}

#[tokio::test]
async fn each_absence_is_a_visible_reason_never_a_zero() {
    let cases = [
        (
            MissingReason::NotInstalled,
            "metrics-server is not installed in this cluster",
        ),
        (
            MissingReason::ApiUnavailable,
            "the metrics API is not answering (metrics-server may be starting or failing)",
        ),
        (
            MissingReason::Forbidden,
            "you are not allowed to read the metrics API",
        ),
    ];
    for (reason, words) in cases {
        let port = FakeMetricsPort::new();
        port.script()
            .node_metrics
            .push_ok(MetricsOutcome::Unavailable(reason));
        port.script()
            .pod_metrics
            .push_ok(MetricsOutcome::Unavailable(reason));
        let expected = format!("metrics unavailable: {words}");
        assert_eq!(text(&node_usage(&port, &cluster(), "n1").await), expected);
        assert_eq!(
            text(&namespace_usage(&port, &cluster(), "shop").await),
            expected
        );
    }
}

#[tokio::test]
async fn a_failed_call_is_shown_with_its_message() {
    let port = FakeMetricsPort::new();
    port.script()
        .node_metrics
        .push_err(OxiError::new(ErrorKind::Timeout, "timed out after 30s"));
    let state = node_usage(&port, &cluster(), "n1").await;
    assert_eq!(
        text(&state),
        "metrics unavailable: the request failed (timed out after 30s)"
    );
}

#[tokio::test]
async fn a_namespace_sums_its_pods_and_asks_for_that_namespace_only() {
    let port = FakeMetricsPort::new();
    port.script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(vec![
            pod_sample("shop", "a", 250_000_000, 100 << 20),
            pod_sample("shop", "b", 150_000_000, 28 << 20),
            pod_sample("elsewhere", "c", 5_000_000_000, 1 << 30),
        ]));
    let usage = ready(namespace_usage(&port, &cluster(), "shop").await);
    assert_eq!(usage.cpu, Ok(q("400m")));
    assert_eq!(usage.memory, Ok(q("128Mi")));
    assert_eq!((usage.pods_reporting, usage.pods_unreadable), (2, 0));
    assert_eq!(
        port.recorded_calls(),
        [MetricsCall::PodMetrics(
            cluster(),
            NamespaceSelection::from_names(["shop"])
        )]
    );
}

#[tokio::test]
async fn a_pod_with_no_reading_is_counted_out_not_as_zero() {
    let mut broken = pod_sample("shop", "b", 0, 0);
    broken.cpu = Reading::Missing(MissingReason::NotYetAvailable);
    broken.memory = Reading::Missing(MissingReason::NotYetAvailable);
    let port = FakeMetricsPort::new();
    port.script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(vec![
            pod_sample("shop", "a", 250_000_000, 100 << 20),
            broken.clone(),
        ]));
    let usage = ready(namespace_usage(&port, &cluster(), "shop").await);
    assert_eq!(usage.cpu, Ok(q("250m")));
    assert_eq!((usage.pods_reporting, usage.pods_unreadable), (1, 1));

    // Every pod unreadable: the sum is missing, with the reason.
    port.script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(vec![broken]));
    let usage = ready(namespace_usage(&port, &cluster(), "shop").await);
    assert_eq!(usage.cpu, Err(MissingReason::NotYetAvailable));
    assert_eq!(usage.memory, Err(MissingReason::NotYetAvailable));
}

#[tokio::test]
async fn a_namespace_with_no_samples_reads_zero_pods_reporting() {
    let port = FakeMetricsPort::new();
    port.script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(Vec::new()));
    let usage = ready(namespace_usage(&port, &cluster(), "shop").await);
    assert_eq!((usage.pods_reporting, usage.pods_unreadable), (0, 0));
}
