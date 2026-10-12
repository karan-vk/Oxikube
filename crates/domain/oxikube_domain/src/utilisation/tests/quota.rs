//! Ephemeral storage in the amounts, the unset-pod counts and the quota caps (E13-S11).

use serde_json::{Value, json};

use super::{container, pod, q};
use crate::resource::Resource;
use crate::utilisation::{
    GpuVendors, QuotaCaps, ResourceAmounts, UnsetPods, aggregate_pod_requests_limits,
};

fn quota(name: &str, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ResourceQuota",
        "metadata": {"name": name, "namespace": "ns"},
        "spec": spec,
        "status": status,
    }))
    .unwrap()
}

fn caps(quota: &Resource) -> QuotaCaps {
    QuotaCaps::from_resource(quota, &GpuVendors::default()).unwrap()
}

#[test]
fn ephemeral_storage_is_summed_like_cpu_and_memory() {
    let p = pod(
        "p",
        "ns",
        None,
        "Running",
        json!({"containers": [
            container("a", json!({"ephemeral-storage": "1Gi"}), json!({"ephemeral-storage": "2Gi"})),
            container("b", json!({"ephemeral-storage": "512Mi"}), json!({})),
        ], "overhead": {"ephemeral-storage": "64Mi"}}),
    );
    let r = aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap();
    assert_eq!(
        r.requests.ephemeral,
        q("1600Mi"),
        "1Gi + 512Mi + 64Mi overhead"
    );
    assert_eq!(
        r.limits.ephemeral,
        q("2112Mi"),
        "overhead rides on a limit that is set"
    );
    assert!(!r.requests.is_zero());
    let both = r.requests.clone() + r.requests;
    assert_eq!(both.ephemeral, q("3200Mi"));
    let p = both.percent_of(&ResourceAmounts {
        ephemeral: q("6400Mi"),
        ..Default::default()
    });
    assert_eq!(p.ephemeral, Some(50.0));
}

#[test]
fn unset_pods_counts_each_resource_on_its_own() {
    let gpus = GpuVendors::default();
    let all = pod(
        "all",
        "ns",
        None,
        "Running",
        json!({"containers": [container(
            "c",
            json!({"cpu": "1", "memory": "1Gi"}),
            json!({"cpu": "2", "memory": "2Gi"}),
        )]}),
    );
    let cpu_only = pod(
        "cpu",
        "ns",
        None,
        "Running",
        json!({"containers": [container("c", json!({"cpu": "1"}), json!({}))]}),
    );
    let none = pod(
        "none",
        "ns",
        None,
        "Running",
        json!({"containers": [container("c", json!({}), json!({}))]}),
    );
    let mut unset = UnsetPods::default();
    for p in [&all, &cpu_only, &none] {
        unset.add(&aggregate_pod_requests_limits(p, &gpus).unwrap());
    }
    assert_eq!(unset.pods, 3);
    assert_eq!((unset.cpu_requests, unset.cpu_limits), (1, 2));
    assert_eq!((unset.memory_requests, unset.memory_limits), (2, 2));
    assert_eq!((unset.ephemeral_requests, unset.ephemeral_limits), (3, 3));
}

#[test]
fn quota_caps_read_hard_with_both_spellings_of_a_request_cap() {
    let a = caps(&quota(
        "a",
        json!({"hard": {"requests.cpu": "4", "limits.memory": "8Gi", "pods": "20",
                        "requests.nvidia.com/gpu": "2", "count/deployments.apps": "5"}}),
        json!({}),
    ));
    assert_eq!(a.requests.cpu, Some(q("4")));
    assert_eq!(a.limits.memory, Some(q("8Gi")));
    assert_eq!(a.pods, Some(20));
    assert_eq!(a.requests.gpu.get("nvidia.com/gpu"), Some(&2));
    assert_eq!(a.requests.memory, None, "nothing caps it: not a zero cap");
    let bare = caps(&quota(
        "b",
        json!({"hard": {"cpu": "3", "memory": "1Gi"}}),
        json!({}),
    ));
    assert_eq!(bare.requests.cpu, Some(q("3")));
    assert_eq!(bare.requests.memory, Some(q("1Gi")));
    // status.hard wins over spec.hard.
    let status = caps(&quota(
        "c",
        json!({"hard": {"requests.cpu": "1"}}),
        json!({"hard": {"requests.cpu": "2"}}),
    ));
    assert_eq!(status.requests.cpu, Some(q("2")));
    let zero = caps(&quota(
        "d",
        json!({"hard": {"requests.cpu": "0"}}),
        json!({}),
    ));
    assert_eq!(zero.requests.cpu, Some(q("0")), "a zero cap is a cap");
}

#[test]
fn the_tightest_cap_wins_and_scoped_quotas_are_left_out() {
    let wide = quota(
        "wide",
        json!({"hard": {"requests.cpu": "8", "pods": "50"}}),
        json!({}),
    );
    let tight = quota(
        "tight",
        json!({"hard": {"requests.cpu": "2", "limits.cpu": "4"}}),
        json!({}),
    );
    let scoped = quota(
        "best-effort",
        json!({"hard": {"pods": "1", "requests.cpu": "1"}, "scopes": ["BestEffort"]}),
        json!({}),
    );
    let (all, left_out) = QuotaCaps::tightest([&wide, &tight, &scoped], &GpuVendors::default());
    assert_eq!(all.requests.cpu, Some(q("2")));
    assert_eq!(all.limits.cpu, Some(q("4")));
    assert_eq!(
        all.pods,
        Some(50),
        "the scoped quota's pods cap is not applied"
    );
    assert_eq!(left_out, 1);
    assert!(!all.is_empty());
    assert!(QuotaCaps::tightest([], &GpuVendors::default()).0.is_empty());
}

#[test]
fn a_non_quota_is_a_wrong_kind() {
    let p = pod("p", "ns", None, "Running", json!({}));
    assert!(QuotaCaps::from_resource(&p, &GpuVendors::default()).is_err());
    let (none, skipped) = QuotaCaps::tightest([&p], &GpuVendors::default());
    assert!(none.is_empty() && skipped == 0);
}
