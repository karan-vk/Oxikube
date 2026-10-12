//! Tests of the live metrics service: the pure parts (cadence, ring, cache, cells) directly, the
//! service against fake sessions, a scripted `FakeMetricsPort` and a manual clock (no sleeps).

mod cadence;
mod columns;
mod ring;
mod service;
mod usage;

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{ContainerUsage, MetricsSample, MetricsSubject, Reading};
use oxikube_ports::MetricsOutcome;

pub(super) const MI: u64 = 1024 * 1024;
/// One millicore, in nanocores.
pub(super) const MILLI: u64 = 1_000_000;

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("a"))
}

pub(super) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

pub(super) fn at(seconds: i64) -> Timestamp {
    t0() + SignedDuration::from_secs(seconds)
}

pub(super) fn node_sample(name: &str, ts: Timestamp, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    )
}

pub(super) fn pod_ref(namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), namespace, name)
}

pub(super) fn pod_sample(
    namespace: &str,
    name: &str,
    ts: Timestamp,
    millis: u64,
    mib: u64,
) -> MetricsSample {
    let mut sample = MetricsSample::new(
        MetricsSubject::Pod {
            pod: pod_ref(namespace, name),
        },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    );
    sample.containers = vec![ContainerUsage {
        name: "app".into(),
        cpu: Reading::Value(millis * MILLI),
        memory: Reading::Value(mib * MI),
    }];
    sample
}

pub(super) fn nodes(samples: Vec<MetricsSample>) -> MetricsOutcome {
    MetricsOutcome::Available(samples)
}

pub(super) fn usage_with_pods(samples: Vec<MetricsSample>) -> Arc<super::ClusterUsage> {
    let usage = Arc::new(super::ClusterUsage::new());
    usage.apply(
        super::MetricsKind::Pods,
        t0(),
        &Ok(MetricsOutcome::Available(samples)),
    );
    usage
}
