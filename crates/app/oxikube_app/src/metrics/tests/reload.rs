//! The aggregator picks up a new `metrics.gpu_vendors` list without being rebuilt.

use serde_json::json;

use super::{cluster, node, pod};
use crate::metrics::{GpuVendorsHandle, UtilisationAggregator};
use oxikube_domain::utilisation::{GpuVendors, GpuVendorsTable};

#[test]
fn a_new_vendor_list_applies_to_the_next_computation() {
    let handle = GpuVendorsHandle::default();
    let agg = UtilisationAggregator::new(handle.clone());
    let a = cluster("a");
    let nodes = [node("n", json!({"cpu": "4", "example.com/fpga": "2"}))];
    let pods = [pod(
        "p",
        "ns",
        Some("n"),
        "Running",
        json!({"cpu": "1", "example.com/fpga": "1"}),
        json!(null),
    )];

    assert!(
        agg.nodes(&a, &nodes, &pods)[0].requests.gpu.is_empty(),
        "not a GPU yet"
    );

    // The binary pushes the resolved settings: the same call now counts the FPGA.
    assert!(handle.set(GpuVendorsTable::new(GpuVendors::new(["example.com/fpga"]))));
    let after = &agg.nodes(&a, &nodes, &pods)[0];
    assert_eq!(after.requests.gpu_count("example.com/fpga"), 1);
    assert_eq!(after.node.allocatable.gpu_count("example.com/fpga"), 2);
    assert_eq!(after.requests_percent().gpu["example.com/fpga"], Some(50.0));

    // Setting the same table again is not a change.
    assert!(!handle.set(GpuVendorsTable::new(GpuVendors::new(["example.com/fpga"]))));
}

#[test]
fn a_cluster_override_applies_to_that_cluster_only() {
    let (a, b) = (cluster("a"), cluster("b"));
    let handle = GpuVendorsHandle::new(
        GpuVendorsTable::new(GpuVendors::default()).with_cluster(b.clone(), GpuVendors::none()),
    );
    let agg = UtilisationAggregator::new(handle);
    let pods = [pod(
        "p",
        "ns",
        Some("n"),
        "Running",
        json!({"nvidia.com/gpu": "1"}),
        json!(null),
    )];
    assert_eq!(agg.namespaces(&a, &pods)[0].requests.gpu_total(), 1);
    assert_eq!(agg.namespaces(&b, &pods)[0].requests.gpu_total(), 0);
}

#[test]
fn the_pod_percents_use_the_clusters_list() {
    use jiff::Timestamp;
    use oxikube_domain::ids::{Gvk, ResourceRef};
    use oxikube_domain::metrics::{MetricsSample, MetricsSubject};

    let a = cluster("a");
    let p = pod(
        "p",
        "ns",
        None,
        "Running",
        json!({"cpu": "200m", "memory": "100Mi"}),
        json!({"cpu": "1"}),
    );
    let r = ResourceRef::namespaced(a.clone(), Gvk::from_api_version("v1", "Pod"), "ns", "p");
    let sample = MetricsSample::new(
        MetricsSubject::Pod { pod: r },
        Timestamp::UNIX_EPOCH,
        None,
        100_000_000,
        50 * 1024 * 1024,
    );
    let u = UtilisationAggregator::default()
        .pod_utilisation(&a, &p, &sample)
        .unwrap();
    assert_eq!(
        (u.cpu_of_requests, u.cpu_of_limits),
        (Some(50.0), Some(10.0))
    );
    assert_eq!(
        (u.memory_of_requests, u.memory_of_limits),
        (Some(50.0), None)
    );
    assert!(
        UtilisationAggregator::default()
            .pod_resources(&a, &node("n", json!(null)))
            .is_err()
    );
}
