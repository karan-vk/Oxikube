//! Node and namespace allocation over a small cluster, with every expected number derived by hand
//! the way `kubectl-view-allocations` (and `kubectl describe node`) would print it.

use proptest::prelude::*;
use serde_json::json;

use super::{cluster, node, pod, q};
use crate::metrics::{UtilisationAggregator, allocate_namespaces, allocate_nodes};
use oxikube_domain::Resource;
use oxikube_domain::utilisation::{GpuVendors, ResourceAmounts};

/// Two nodes, `gpu-1` with 4 NVIDIA GPUs. Pods:
///
/// | pod | ns | node | phase | cpu req / lim | mem req / lim | gpu |
/// |---|---|---|---|---|---|---|
/// | a | web | n1 | Running | 500m / 1 | 256Mi / 512Mi | |
/// | b | web | n1 | Running | 250m / - | 128Mi / - | |
/// | c | db | n1 | Pending | 1 / 2 | 1Gi / 2Gi | |
/// | d | db | n2 | Running | 2 / 4 | 4Gi / 8Gi | |
/// | e | batch | gpu-1 | Running | 1 / 1 | 1Gi / 1Gi | 2 |
/// | f | batch | n1 | Succeeded | 8 / 8 | 8Gi / 8Gi | (terminated: ignored) |
/// | g | web | - | Pending | 100m / - | 64Mi / - | (unscheduled: namespace only) |
/// | h | web | n9 | Running | 3 / - | 3Gi / - | (node not in the list) |
fn cluster_fixture() -> (Vec<Resource>, Vec<Resource>) {
    let nodes = vec![
        node("n1", json!({"cpu": "4", "memory": "8Gi", "pods": "110"})),
        node("n2", json!({"cpu": "8", "memory": "16Gi", "pods": "110"})),
        node(
            "gpu-1",
            json!({"cpu": "16", "memory": "64Gi", "pods": "110", "nvidia.com/gpu": "4"}),
        ),
    ];
    let pods = vec![
        pod(
            "a",
            "web",
            Some("n1"),
            "Running",
            json!({"cpu": "500m", "memory": "256Mi"}),
            json!({"cpu": "1", "memory": "512Mi"}),
        ),
        pod(
            "b",
            "web",
            Some("n1"),
            "Running",
            json!({"cpu": "250m", "memory": "128Mi"}),
            json!(null),
        ),
        pod(
            "c",
            "db",
            Some("n1"),
            "Pending",
            json!({"cpu": "1", "memory": "1Gi"}),
            json!({"cpu": "2", "memory": "2Gi"}),
        ),
        pod(
            "d",
            "db",
            Some("n2"),
            "Running",
            json!({"cpu": "2", "memory": "4Gi"}),
            json!({"cpu": "4", "memory": "8Gi"}),
        ),
        pod(
            "e",
            "batch",
            Some("gpu-1"),
            "Running",
            json!({"cpu": "1", "memory": "1Gi", "nvidia.com/gpu": "2"}),
            json!({"cpu": "1", "memory": "1Gi", "nvidia.com/gpu": "2"}),
        ),
        pod(
            "f",
            "batch",
            Some("n1"),
            "Succeeded",
            json!({"cpu": "8", "memory": "8Gi"}),
            json!({"cpu": "8", "memory": "8Gi"}),
        ),
        pod(
            "g",
            "web",
            None,
            "Pending",
            json!({"cpu": "100m", "memory": "64Mi"}),
            json!(null),
        ),
        pod(
            "h",
            "web",
            Some("n9"),
            "Running",
            json!({"cpu": "3", "memory": "3Gi"}),
            json!(null),
        ),
    ];
    (nodes, pods)
}

#[test]
fn nodes_sum_the_pods_booked_on_them() {
    let (nodes, pods) = cluster_fixture();
    let out = allocate_nodes(&nodes, &pods, &GpuVendors::default());
    let names: Vec<&str> = out.iter().map(|n| &*n.node.name).collect();
    assert_eq!(names, ["n1", "n2", "gpu-1"], "input order");

    // n1: a + b + c = 500m + 250m + 1 = 1750m cpu; 256Mi + 128Mi + 1Gi = 1408Mi mem;
    // limits: 1 + 2 = 3 cpu (b has none); 512Mi + 2Gi = 2560Mi. 3 pods (f is Succeeded).
    let n1 = &out[0];
    assert_eq!(n1.requests.cpu, q("1750m"));
    assert_eq!(n1.requests.memory, q("1408Mi"));
    assert_eq!(n1.limits.cpu, q("3"));
    assert_eq!(n1.limits.memory, q("2560Mi"));
    assert_eq!(n1.pods, 3);
    // 1750m of 4 cores = 43.75 %; 1408Mi of 8Gi = 17.1875 %; limits 3 of 4 = 75 %.
    let p = n1.requests_percent();
    assert_eq!((p.cpu, p.memory), (Some(43.75), Some(17.1875)));
    assert_eq!(n1.limits_percent().cpu, Some(75.0));
    assert_eq!(n1.pods_percent(), Some(300.0 / 110.0));

    // n2: d alone: 2 of 8 cores = 25 %; 4Gi of 16Gi = 25 %; limits 4 / 8 = 50 %.
    let n2 = &out[1];
    assert_eq!(n2.requests_percent().cpu, Some(25.0));
    assert_eq!(n2.requests_percent().memory, Some(25.0));
    assert_eq!(n2.limits_percent().cpu, Some(50.0));

    // gpu-1: e: 2 of 4 GPUs = 50 %.
    let gpu = &out[2];
    assert_eq!(gpu.requests.gpu_count("nvidia.com/gpu"), 2);
    assert_eq!(gpu.requests_percent().gpu["nvidia.com/gpu"], Some(50.0));
    assert_eq!(gpu.requests_percent().cpu, Some(6.25));
}

#[test]
fn a_node_without_allocatable_has_dashes_not_zero_or_infinity() {
    let nodes = [node("bare", json!(null))];
    let pods = [pod(
        "a",
        "ns",
        Some("bare"),
        "Running",
        json!({"cpu": "1"}),
        json!(null),
    )];
    let out = allocate_nodes(&nodes, &pods, &GpuVendors::default());
    assert_eq!(out[0].requests.cpu, q("1"));
    assert_eq!(out[0].requests_percent().cpu, None);
    assert_eq!(out[0].pods_percent(), None);
}

#[test]
fn namespaces_count_unscheduled_pods_and_skip_terminated_ones() {
    let (_, pods) = cluster_fixture();
    let out = allocate_namespaces(&pods, &GpuVendors::default());
    let names: Vec<&str> = out.iter().map(|n| &*n.namespace).collect();
    assert_eq!(names, ["batch", "db", "web"], "sorted");

    // web: a + b + g + h = 500m + 250m + 100m + 3 = 3850m; 256Mi + 128Mi + 64Mi + 3Gi = 3520Mi.
    let web = &out[2];
    assert_eq!(web.pods, 4);
    assert_eq!(web.requests.cpu, q("3850m"));
    assert_eq!(web.requests.memory, q("3520Mi"));
    assert_eq!(web.limits.cpu, q("1"));

    // db: c (Pending, scheduled) + d = 3 cpu, 5Gi.
    assert_eq!(out[1].requests.cpu, q("3"));
    assert_eq!(out[1].requests.memory, q("5Gi"));

    // batch: only e; f (Succeeded) is not counted.
    assert_eq!(out[0].pods, 1);
    assert_eq!(out[0].requests.gpu_count("nvidia.com/gpu"), 2);
}

#[test]
fn a_quota_comparison_is_a_percent_of_the_hard_amounts() {
    let (_, pods) = cluster_fixture();
    let db = &allocate_namespaces(&pods, &GpuVendors::default())[1];
    let hard = ResourceAmounts {
        cpu: q("6"),
        memory: q("10Gi"),
        ..Default::default()
    };
    let p = db.requests_vs(&hard);
    assert_eq!((p.cpu, p.memory), (Some(50.0), Some(50.0)));
    let uncapped = db.limits_vs(&ResourceAmounts::zero());
    assert_eq!(
        (uncapped.cpu, uncapped.memory),
        (None, None),
        "no hard amount: a dash"
    );
}

#[test]
fn mixed_lists_are_tolerated() {
    let (nodes, pods) = cluster_fixture();
    let mixed: Vec<&Resource> = nodes.iter().chain(pods.iter()).collect();
    let out = allocate_nodes(
        mixed.iter().copied(),
        mixed.iter().copied(),
        &GpuVendors::default(),
    );
    assert_eq!(out.len(), 3);
    assert_eq!(
        allocate_namespaces(mixed.iter().copied(), &GpuVendors::default()).len(),
        3
    );
}

#[test]
fn the_aggregator_reads_the_vendor_list_of_the_cluster() {
    let (nodes, pods) = cluster_fixture();
    let agg = UtilisationAggregator::default();
    let out = agg.nodes(&cluster("a"), &nodes, &pods);
    assert_eq!(out[2].requests.gpu_count("nvidia.com/gpu"), 2);
    assert_eq!(
        agg.namespaces(&cluster("a"), &pods)[0].requests.gpu_total(),
        2
    );
}

fn arbitrary_pod() -> impl Strategy<Value = Resource> {
    (
        0u8..4,
        prop::option::of(0u8..3),
        prop_oneof![
            Just("Running"),
            Just("Pending"),
            Just("Succeeded"),
            Just("Failed")
        ],
        0u32..4000,
        0u32..4096,
        0u8..3,
    )
        .prop_map(|(ns, node, phase, cpu, mem, gpu)| {
            let node_name = node.map(|n| format!("n{n}"));
            let mut requests = json!({"cpu": format!("{cpu}m"), "memory": format!("{mem}Mi")});
            if gpu > 0 {
                requests["nvidia.com/gpu"] = json!(gpu.to_string());
            }
            pod(
                &format!("p{cpu}-{mem}"),
                &format!("ns{ns}"),
                node_name.as_deref(),
                phase,
                requests,
                json!(null),
            )
        })
}

proptest! {
    /// The order of the pods in the snapshot changes nothing, and the books balance: every
    /// pod that holds resources is in exactly one namespace, and the node sums plus the pods on
    /// unknown or no node make the namespace total.
    #[test]
    fn sums_are_order_independent_and_balance(mut pods in prop::collection::vec(arbitrary_pod(), 0..24), k in 0usize..24) {
        let nodes: Vec<Resource> = (0..3).map(|i| node(&format!("n{i}"), json!({"cpu": "8", "memory": "16Gi"}))).collect();
        let gpus = GpuVendors::default();
        let nodes_before = allocate_nodes(&nodes, &pods, &gpus);
        let ns_before = allocate_namespaces(&pods, &gpus);
        if !pods.is_empty() { let n = k % pods.len(); pods.rotate_left(n); }
        pods.reverse();
        prop_assert_eq!(&allocate_nodes(&nodes, &pods, &gpus), &nodes_before);
        prop_assert_eq!(&allocate_namespaces(&pods, &gpus), &ns_before);

        let ns_total: ResourceAmounts = ns_before.iter().map(|n| &n.requests).sum();
        let on_nodes: ResourceAmounts = nodes_before.iter().map(|n| &n.requests).sum();
        // What is not on a listed node is scheduled nowhere (the generator only uses n0..n2,
        // all listed) or unscheduled, so the node total never exceeds the namespace total.
        prop_assert!(on_nodes.cpu <= ns_total.cpu && on_nodes.memory <= ns_total.memory);
        let counted: u32 = ns_before.iter().map(|n| n.pods).sum();
        let holding = pods.iter().filter(|p| {
            !matches!(p.get_str("/status/phase"), Some("Succeeded" | "Failed"))
        }).count() as u32;
        prop_assert_eq!(counted, holding);
    }
}
