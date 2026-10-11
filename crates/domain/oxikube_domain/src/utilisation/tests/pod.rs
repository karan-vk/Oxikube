//! Pod requests and limits: containers, init containers, sidecars, overhead, GPUs.

use serde_json::json;

use super::{container, node, pod, q, sidecar};
use crate::utilisation::{
    DEFAULT_GPU_VENDORS, GpuVendors, GpuVendorsTable, NodeAllocatable, PodAllocation,
    ResourceAmounts, aggregate_pod_requests_limits,
};
use crate::view::PodPhase;

fn agg(spec: serde_json::Value) -> crate::utilisation::PodResources {
    let pod = pod("p", "ns", None, "Running", spec);
    aggregate_pod_requests_limits(&pod, &GpuVendors::default()).unwrap()
}

#[test]
fn containers_are_summed() {
    let r = agg(json!({"containers": [
        container("a", json!({"cpu": "250m", "memory": "128Mi"}), json!({"cpu": "500m", "memory": "256Mi"})),
        container("b", json!({"cpu": "0.25", "memory": "128Mi"}), json!({"cpu": "1", "memory": "1Gi"})),
    ]}));
    assert_eq!(r.requests.cpu, q("500m"));
    assert_eq!(r.requests.memory, q("256Mi"));
    assert_eq!(r.limits.cpu, q("1500m"));
    assert_eq!(r.limits.memory, q("1280Mi"));
}

#[test]
fn a_pod_without_requests_or_limits_is_zero() {
    let r = agg(json!({"containers": [container("a", json!(null), json!(null))]}));
    assert!(r.is_empty());
    assert!(r.requests.cpu.is_zero() && r.limits.memory.is_zero());
}

#[test]
fn a_limit_without_a_request_only_sets_the_limit() {
    // The API server copies the limit into the request at admission; the stored spec already
    // says so, and nothing is defaulted here.
    let r = agg(json!({"containers": [container("a", json!(null), json!({"cpu": "1"}))]}));
    assert!(r.requests.cpu.is_zero());
    assert_eq!(r.limits.cpu, q("1"));
}

#[test]
fn a_large_init_container_raises_the_pod_to_its_peak() {
    // Kubernetes: max(init, sum(containers)). Init 2 cores beats containers 300m + 200m.
    let r = agg(json!({
        "initContainers": [container("init", json!({"cpu": "2", "memory": "64Mi"}), json!(null))],
        "containers": [
            container("a", json!({"cpu": "300m", "memory": "512Mi"}), json!(null)),
            container("b", json!({"cpu": "200m", "memory": "512Mi"}), json!(null)),
        ],
    }));
    assert_eq!(r.requests.cpu, q("2"));
    assert_eq!(
        r.requests.memory,
        q("1Gi"),
        "memory: the containers' sum is the larger"
    );
}

#[test]
fn init_containers_run_one_at_a_time() {
    let r = agg(json!({
        "initContainers": [
            container("i1", json!({"cpu": "1"}), json!(null)),
            container("i2", json!({"cpu": "3"}), json!(null)),
        ],
        "containers": [container("a", json!({"cpu": "2"}), json!(null))],
    }));
    assert_eq!(
        r.requests.cpu,
        q("3"),
        "the largest init container, not their sum"
    );
}

#[test]
fn sidecars_add_to_the_running_total_and_to_later_init_containers() {
    // Order: sidecar s1 (100m), init i1 (1), sidecar s2 (200m), init i2 (500m); app 300m.
    // Peaks: s1 alone 100m; i1 = 1 + 100m = 1100m; s2 = 100m + 200m = 300m;
    // i2 = 500m + 300m = 800m. Running total = 300m + 100m + 200m = 600m.
    // Effective = max(600m, 1100m) = 1100m.
    let r = agg(json!({
        "initContainers": [
            sidecar("s1", json!({"cpu": "100m"}), json!(null)),
            container("i1", json!({"cpu": "1"}), json!(null)),
            sidecar("s2", json!({"cpu": "200m"}), json!(null)),
            container("i2", json!({"cpu": "500m"}), json!(null)),
        ],
        "containers": [container("a", json!({"cpu": "300m"}), json!(null))],
    }));
    assert_eq!(r.requests.cpu, q("1100m"));

    // Without the big init container the sidecars simply add up.
    let r = agg(json!({
        "initContainers": [sidecar("s1", json!({"cpu": "100m"}), json!(null))],
        "containers": [container("a", json!({"cpu": "300m"}), json!(null))],
    }));
    assert_eq!(r.requests.cpu, q("400m"));
}

#[test]
fn overhead_is_added_to_requests_and_to_set_limits() {
    let r = agg(json!({
        "overhead": {"cpu": "250m", "memory": "120Mi"},
        "containers": [container("a", json!({"cpu": "1", "memory": "1Gi"}), json!({"memory": "2Gi"}))],
    }));
    assert_eq!(r.requests.cpu, q("1250m"));
    assert_eq!(r.requests.memory, q("1144Mi"));
    assert_eq!(
        r.limits.memory,
        q("2168Mi"),
        "overhead rides on a limit that is set"
    );
    assert!(r.limits.cpu.is_zero(), "and does not invent a CPU limit");
}

#[test]
fn pod_level_resources_replace_the_container_sum_for_what_they_set() {
    let r = agg(json!({
        "resources": {"requests": {"cpu": "4"}, "limits": {"cpu": "8"}},
        "containers": [container("a", json!({"cpu": "1", "memory": "1Gi"}), json!({"memory": "2Gi"}))],
    }));
    assert_eq!(r.requests.cpu, q("4"));
    assert_eq!(
        r.requests.memory,
        q("1Gi"),
        "memory: not set pod-level, the containers' sum"
    );
    assert_eq!(r.limits.cpu, q("8"));
    assert_eq!(r.limits.memory, q("2Gi"));
}

#[test]
fn bare_numbers_and_junk_are_read_without_failing() {
    let r = agg(json!({"containers": [
        container("a", json!({"cpu": 2, "memory": "oops"}), json!({"cpu": -1})),
        json!("not an object"),
    ]}));
    assert_eq!(r.requests.cpu, q("2"));
    assert!(r.requests.memory.is_zero());
    assert!(r.limits.cpu.is_zero(), "a negative quantity is ignored");
}

#[test]
fn a_wrong_kind_is_an_error() {
    let n = node("n", json!({}));
    assert!(aggregate_pod_requests_limits(&n, &GpuVendors::default()).is_err());
    assert!(PodAllocation::from_resource(&n, &GpuVendors::default()).is_err());
    assert!(
        NodeAllocatable::from_resource(
            &pod("p", "ns", None, "Running", json!({})),
            &GpuVendors::default()
        )
        .is_err()
    );
}

#[test]
fn gpus_are_whole_counts_of_the_configured_vendors_only() {
    let spec = json!({"containers": [
        container("a", json!({"nvidia.com/gpu": "1", "example.com/fpga": "2"}), json!({"nvidia.com/gpu": "1", "amd.com/gpu": "2"})),
        container("b", json!({"nvidia.com/gpu": 2}), json!(null)),
    ]});
    let p = pod("p", "ns", None, "Running", spec);

    let default = aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap();
    assert_eq!(default.requests.gpu_count("nvidia.com/gpu"), 3);
    assert_eq!(
        default.requests.gpu_count("example.com/fpga"),
        0,
        "not a configured vendor"
    );
    assert_eq!(default.limits.gpu_count("amd.com/gpu"), 2);
    assert_eq!(default.requests.gpu_total(), 3);

    let custom = GpuVendors::new(["example.com/fpga"]);
    let r = aggregate_pod_requests_limits(&p, &custom).unwrap();
    assert_eq!(r.requests.gpu_count("example.com/fpga"), 2);
    assert_eq!(
        r.requests.gpu_count("nvidia.com/gpu"),
        0,
        "dropped from the list, no longer a GPU"
    );
    assert!(r.limits.gpu.is_empty());
}

#[test]
fn a_fractional_gpu_rounds_up_and_a_sidecar_gpu_adds_up() {
    let r = agg(json!({
        "initContainers": [sidecar("s", json!({"nvidia.com/gpu": "1"}), json!(null))],
        "containers": [container("a", json!({"nvidia.com/gpu": "500m"}), json!(null))],
    }));
    assert_eq!(r.requests.gpu_count("nvidia.com/gpu"), 2);
}

#[test]
fn gpu_vendors_normalise_and_the_table_falls_back_to_its_default() {
    let v = GpuVendors::new([" nvidia.com/gpu ", "", "nvidia.com/gpu", "amd.com/gpu"]);
    let names: Vec<&str> = v.names().iter().map(|n| &**n).collect();
    assert_eq!(names, ["nvidia.com/gpu", "amd.com/gpu"]);
    assert!(v.contains("amd.com/gpu") && !v.contains("x"));
    assert!(GpuVendors::none().is_empty());
    let default_list = GpuVendors::default();
    let defaults: Vec<&str> = default_list.names().iter().map(|n| &**n).collect();
    assert_eq!(defaults, DEFAULT_GPU_VENDORS);

    let a = crate::ids::ClusterId::new("f", &crate::ids::ContextName::new("a"));
    let b = crate::ids::ClusterId::new("f", &crate::ids::ContextName::new("b"));
    let table = GpuVendorsTable::new(GpuVendors::default())
        .with_cluster(a.clone(), GpuVendors::new(["x/gpu"]));
    assert!(table.for_cluster(&a).contains("x/gpu"));
    assert_eq!(table.for_cluster(&b), table.default_vendors());
}

#[test]
fn allocation_records_where_a_pod_runs_and_whether_it_still_holds_resources() {
    let spec = json!({"containers": [container("a", json!({"cpu": "1"}), json!(null))]});
    let gpus = GpuVendors::default();
    for (phase, node_name, holds, scheduled) in [
        ("Running", Some("n1"), true, true),
        ("Pending", Some("n1"), true, true),
        ("Pending", None, true, false),
        ("Unknown", Some("n1"), true, true),
        ("Succeeded", Some("n1"), false, true),
        ("Failed", Some("n1"), false, true),
    ] {
        let a =
            PodAllocation::from_resource(&pod("p", "ns", node_name, phase, spec.clone()), &gpus)
                .unwrap();
        assert_eq!(a.holds_resources(), holds, "{phase}");
        assert_eq!(a.is_scheduled(), scheduled, "{phase} {node_name:?}");
        assert_eq!(a.namespace.as_deref(), Some("ns"));
        assert_eq!(a.resources.requests.cpu, q("1"));
    }
    let a = PodAllocation::from_resource(&pod("p", "ns", None, "Bogus", spec), &gpus).unwrap();
    assert_eq!(a.phase, PodPhase::Unknown);
}

#[test]
fn node_allocatable_reads_cpu_memory_pods_and_gpus() {
    let n = node(
        "n1",
        json!({"cpu": "3920m", "memory": "16Gi", "pods": "110", "nvidia.com/gpu": "4", "ephemeral-storage": "100Gi"}),
    );
    let a = NodeAllocatable::from_resource(&n, &GpuVendors::default()).unwrap();
    assert_eq!(&*a.name, "n1");
    assert_eq!(a.allocatable.cpu, q("3920m"));
    assert_eq!(a.allocatable.memory, q("16Gi"));
    assert_eq!(a.allocatable.gpu_count("nvidia.com/gpu"), 4);
    assert_eq!(a.max_pods, Some(110));
    let none =
        NodeAllocatable::from_resource(&node("n2", json!(null)), &GpuVendors::default()).unwrap();
    assert!(none.allocatable.is_zero());
    assert_eq!(none.max_pods, None);
}

#[test]
fn amounts_add_saturate_and_compare() {
    let mut a = ResourceAmounts {
        cpu: q("1"),
        memory: q("1Gi"),
        ..Default::default()
    };
    a.gpu.insert("nvidia.com/gpu".into(), 1);
    let sum = a.clone() + a.clone();
    assert_eq!(sum.cpu, q("2"));
    assert_eq!(sum.gpu_count("nvidia.com/gpu"), 2);
    assert!(ResourceAmounts::zero().is_zero() && !sum.is_zero());
    let total: ResourceAmounts = [&a, &a, &a].into_iter().sum();
    assert_eq!(total.cpu, q("3"));
    let mut huge = ResourceAmounts {
        cpu: crate::quantity::Quantity::MAX,
        ..Default::default()
    };
    huge += &huge.clone();
    assert_eq!(
        huge.cpu,
        crate::quantity::Quantity::MAX,
        "saturates, never panics"
    );
}
