//! `percent`, `Utilisation` and `AmountPercents`.

use jiff::Timestamp;
use serde_json::json;

use super::{container, pod, q};
use crate::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use crate::metrics::{MetricsSample, MetricsSubject, MissingReason};
use crate::quantity::Quantity;
use crate::utilisation::{
    GpuVendors, ResourceAmounts, Utilisation, aggregate_pod_requests_limits, percent,
};

#[test]
fn percent_is_exact_and_none_without_a_base() {
    assert_eq!(percent(&q("250m"), &q("1")), Some(25.0));
    assert_eq!(percent(&q("512Mi"), &q("1Gi")), Some(50.0));
    assert_eq!(
        percent(&q("2"), &q("1")),
        Some(200.0),
        "over 100 is allowed"
    );
    assert_eq!(percent(&Quantity::ZERO, &q("1")), Some(0.0));
    assert_eq!(percent(&q("1"), &Quantity::ZERO), None);
    assert_eq!(percent(&q("-1"), &q("1")), None);
    assert_eq!(percent(&q("1"), &q("-1")), None);
    assert_eq!(
        percent(&q("1m"), &q("3m")).map(|p| (p * 1e6).round()),
        Some(33_333_333.0)
    );
}

#[test]
fn percent_handles_values_beyond_the_integer_fast_path() {
    let huge = Quantity::parse("1000000000000000000000000000000E").unwrap_or(Quantity::MAX);
    assert!(percent(&huge, &huge).is_some_and(|p| (p - 100.0).abs() < 1e-6));
}

fn sample(cpu_nanos: u64, mem_bytes: u64) -> MetricsSample {
    let cluster = ClusterId::new("k", &ContextName::new("c"));
    let r = ResourceRef::namespaced(cluster, Gvk::from_api_version("v1", "Pod"), "ns", "p");
    MetricsSample::new(
        MetricsSubject::Pod { pod: r },
        Timestamp::UNIX_EPOCH,
        None,
        cpu_nanos,
        mem_bytes,
    )
}

#[test]
fn utilisation_gives_the_four_k9s_columns() {
    let p = pod(
        "p",
        "ns",
        None,
        "Running",
        json!({"containers": [
            container("a", json!({"cpu": "200m", "memory": "100Mi"}), json!({"cpu": "1", "memory": "400Mi"})),
        ]}),
    );
    let res = aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap();
    // 100m of 200m = 50 %, of 1 core = 10 %; 50Mi of 100Mi = 50 %, of 400Mi = 12.5 %.
    let u = Utilisation::of_sample(&sample(100_000_000, 50 * 1024 * 1024), &res);
    assert_eq!(u.cpu_of_requests, Some(50.0));
    assert_eq!(u.cpu_of_limits, Some(10.0));
    assert_eq!(u.memory_of_requests, Some(50.0));
    assert_eq!(u.memory_of_limits, Some(12.5));
}

#[test]
fn no_request_or_limit_and_no_reading_are_dashes() {
    let p = pod(
        "p",
        "ns",
        None,
        "Running",
        json!({"containers": [
            container("a", json!({"cpu": "200m"}), json!(null)),
        ]}),
    );
    let res = aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap();
    let u = Utilisation::of_sample(&sample(100_000_000, 1), &res);
    assert_eq!(u.cpu_of_requests, Some(50.0));
    assert_eq!(u.cpu_of_limits, None, "no CPU limit");
    assert_eq!(u.memory_of_requests, None, "no memory request");
    assert_eq!(u.memory_of_limits, None);

    let missing = MetricsSample::missing(
        sample(0, 0).subject,
        Timestamp::UNIX_EPOCH,
        MissingReason::NotInstalled,
    );
    assert_eq!(
        Utilisation::of_sample(&missing, &res),
        Utilisation::default()
    );
}

#[test]
fn amounts_as_percent_of_a_base() {
    let mut used = ResourceAmounts {
        cpu: q("1"),
        memory: q("2Gi"),
        ..Default::default()
    };
    used.gpu.insert("nvidia.com/gpu".into(), 1);
    used.gpu.insert("amd.com/gpu".into(), 1);
    let mut base = ResourceAmounts {
        cpu: q("4"),
        memory: Quantity::ZERO,
        ..Default::default()
    };
    base.gpu.insert("nvidia.com/gpu".into(), 4);
    let p = used.percent_of(&base);
    assert_eq!(p.cpu, Some(25.0));
    assert_eq!(p.memory, None);
    assert_eq!(p.gpu["nvidia.com/gpu"], Some(25.0));
    assert_eq!(
        p.gpu["amd.com/gpu"], None,
        "requested but the node has none"
    );
}
