//! Node detail view-models (E12-S09): taints, instance type and zone, capacity rows, images, the
//! demand of the pods on a node and the allocation maths with `m`, `Ki`, `Mi`, `Gi` and decimals.

use oxikube_domain::view::TaintEffect;
use oxikube_domain::{Demand, NodeAllocation, NodeDetail, NodeSummary, PodPlacement, Resource};
use serde_json::{Value, json};

fn node(labels: Value, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": "n1", "labels": labels},
        "spec": spec, "status": status
    }))
    .expect("a Node resource")
}

fn pod(spec: Value, phase: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "p", "namespace": "ns"},
        "spec": spec, "status": {"phase": phase}
    }))
    .expect("a Pod resource")
}

fn container(requests: Value, limits: Value) -> Value {
    json!({"name": "c", "image": "i", "resources": {"requests": requests, "limits": limits}})
}

// --- taints, labels -------------------------------------------------------------------------

#[test]
fn taints_show_key_value_and_effect_like_kubectl() {
    let res = node(
        json!({}),
        json!({"taints": [
            {"key": "node-role.kubernetes.io/control-plane", "effect": "NoSchedule"},
            {"key": "dedicated", "value": "gpu", "effect": "NoExecute"},
            {"key": "soft", "value": "x", "effect": "PreferNoSchedule"},
            {"key": "future", "effect": "SomethingNew"},
            {"effect": "NoSchedule"}
        ]}),
        json!({}),
    );
    let summary = NodeSummary::from_resource(&res).unwrap();
    let shown: Vec<String> = summary.taints.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "node-role.kubernetes.io/control-plane:NoSchedule",
            "dedicated=gpu:NoExecute",
            "soft=x:PreferNoSchedule",
            "future:SomethingNew",
        ]
    );
    let effects: Vec<_> = summary.taints.iter().map(|t| t.effect).collect();
    assert_eq!(
        effects,
        [
            Some(TaintEffect::NoSchedule),
            Some(TaintEffect::NoExecute),
            Some(TaintEffect::PreferNoSchedule),
            None
        ]
    );
}

#[test]
fn a_cordoned_node_carries_the_unschedulable_taint_and_the_flag() {
    let res = node(
        json!({}),
        json!({"unschedulable": true, "taints": [
            {"key": "node.kubernetes.io/unschedulable", "effect": "NoSchedule"}
        ]}),
        json!({"conditions": [{"type": "Ready", "status": "True"}]}),
    );
    let summary = NodeSummary::from_resource(&res).unwrap();
    assert!(!summary.schedulable);
    assert_eq!(&*summary.status, "Ready,SchedulingDisabled");
    assert!(summary.taints[0].is_unschedulable());
}

#[test]
fn instance_type_and_zone_come_from_the_well_known_labels_current_first() {
    let current = NodeSummary::from_resource(&node(
        json!({
            "node.kubernetes.io/instance-type": "m5.large",
            "beta.kubernetes.io/instance-type": "old",
            "topology.kubernetes.io/zone": "eu-west-1a"
        }),
        json!({}),
        json!({}),
    ))
    .unwrap();
    assert_eq!(current.instance_type.as_deref(), Some("m5.large"));
    assert_eq!(current.zone.as_deref(), Some("eu-west-1a"));

    let legacy = NodeSummary::from_resource(&node(
        json!({
            "beta.kubernetes.io/instance-type": "n1-standard-4",
            "failure-domain.beta.kubernetes.io/zone": "us-central1-b"
        }),
        json!({}),
        json!({}),
    ))
    .unwrap();
    assert_eq!(legacy.instance_type.as_deref(), Some("n1-standard-4"));
    assert_eq!(legacy.zone.as_deref(), Some("us-central1-b"));

    let none = NodeSummary::from_resource(&node(json!({}), json!({}), json!({}))).unwrap();
    assert_eq!((none.instance_type, none.zone), (None, None));
}

// --- detail ------------------------------------------------------------------------------

#[test]
fn the_detail_lists_capacity_beside_allocatable_images_by_size_and_system_info() {
    let res = node(
        json!({}),
        json!({"podCIDR": "10.0.0.0/24", "providerID": "aws:///eu-west-1a/i-1"}),
        json!({
            "capacity": {"cpu": "4", "memory": "16Gi", "pods": "110",
                "hugepages-2Mi": "0", "ephemeral-storage": "100Gi", "vendor.com/gpu": "1"},
            "allocatable": {"cpu": "3920m", "memory": "15Gi", "pods": "110",
                "hugepages-2Mi": "0", "ephemeral-storage": "90Gi", "vendor.com/gpu": "1"},
            "addresses": [{"type": "InternalIP", "address": "10.0.0.5"}],
            "nodeInfo": {"kubeletVersion": "v1.33.1", "osImage": "Debian", "kernelVersion": "6.8"},
            "images": [
                {"names": ["sha256:aaa", "docker.io/small:1"], "sizeBytes": 1048576},
                {"names": ["docker.io/big@sha256:bbb", "docker.io/big:2"], "sizeBytes": 2147483648_u64},
                {"names": ["docker.io/unknown:3"]}
            ]
        }),
    );
    let detail = NodeDetail::from_resource(&res);
    let names: Vec<&str> = detail.capacity.iter().map(|r| &*r.name).collect();
    assert_eq!(
        names,
        [
            "cpu",
            "memory",
            "pods",
            "ephemeral-storage",
            "hugepages-2Mi",
            "vendor.com/gpu"
        ]
    );
    assert_eq!(detail.capacity[0].capacity.as_deref(), Some("4"));
    assert_eq!(detail.capacity[0].allocatable.as_deref(), Some("3920m"));

    let images: Vec<(&str, String)> = detail
        .images
        .iter()
        .map(|i| (&*i.name, i.size_text()))
        .collect();
    assert_eq!(
        images,
        [
            ("docker.io/big:2", "2 GiB".to_owned()),
            ("sha256:aaa", "1 MiB".to_owned()),
            ("docker.io/unknown:3", String::new()),
        ]
    );
    let system: Vec<&str> = detail.system.iter().map(|(label, _)| *label).collect();
    assert_eq!(system, ["Kernel version", "OS image", "Kubelet version"]);
    assert_eq!(detail.pod_cidr.as_deref(), Some("10.0.0.0/24"));
    assert_eq!(detail.provider_id.as_deref(), Some("aws:///eu-west-1a/i-1"));
    assert_eq!(&*detail.addresses[0].1, "10.0.0.5");
}

#[test]
fn a_node_without_status_has_an_empty_detail() {
    let detail = NodeDetail::from_resource(&node(json!({}), json!({}), Value::Null));
    assert_eq!(detail, NodeDetail::default());
}

// --- demand ------------------------------------------------------------------------------

#[test]
fn a_pod_demand_sums_its_containers_with_every_unit() {
    let spec = json!({
        "nodeName": "n1",
        "containers": [
            container(json!({"cpu": "250m", "memory": "64Mi"}), json!({"cpu": "500m", "memory": "128Mi"})),
            container(json!({"cpu": "0.5", "memory": "1Gi"}), json!({"cpu": "1", "memory": "1.5Gi"})),
            container(json!({"cpu": "1500u", "memory": "512Ki"}), json!({})),
        ]
    });
    let placement = PodPlacement::of_pod(&pod(spec, "Running")).unwrap();
    assert_eq!(&*placement.node, "n1");
    let d = placement.demand;
    assert_eq!(d.pods, 1);
    // 250m + 500m + 1500u (rounded away from zero: 2m)
    assert_eq!(d.cpu_requests, 752);
    assert_eq!(d.cpu_limits, 1500);
    assert_eq!(
        d.memory_requests,
        64 * 1024 * 1024 + 1024 * 1024 * 1024 + 512 * 1024
    );
    assert_eq!(d.memory_limits, 128 * 1024 * 1024 + 1610612736);
}

#[test]
fn a_missing_request_defaults_to_the_limit_like_the_api_server() {
    let spec = json!({"nodeName": "n1", "containers": [
        {"name": "c", "resources": {"limits": {"cpu": "2", "memory": "1Gi"}}}
    ]});
    let d = PodPlacement::of_pod(&pod(spec, "Pending")).unwrap().demand;
    assert_eq!((d.cpu_requests, d.cpu_limits), (2000, 2000));
    assert_eq!(d.memory_requests, 1 << 30);
}

#[test]
fn init_containers_raise_the_pod_to_the_largest_and_sidecars_add() {
    // Containers need 100m; a plain init container needs 400m (peak 400m); a sidecar needs 50m
    // and is added to the sum (150m) and to the init containers after it.
    let spec = json!({
        "nodeName": "n1",
        "containers": [container(json!({"cpu": "100m", "memory": "10Mi"}), json!({}))],
        "initContainers": [
            container(json!({"cpu": "400m", "memory": "1Mi"}), json!({})),
            {"name": "side", "restartPolicy": "Always",
                "resources": {"requests": {"cpu": "50m", "memory": "5Mi"}}},
            container(json!({"cpu": "300m", "memory": "1Mi"}), json!({})),
        ],
        "overhead": {"cpu": "10m", "memory": "1Mi"}
    });
    let d = PodPlacement::of_pod(&pod(spec, "Running")).unwrap().demand;
    // peak(init) = max(400m, 300m + 50m sidecar) = 400m; sum = 100m + 50m = 150m; + overhead 10m
    assert_eq!(d.cpu_requests, 410);
    // memory: sum = 10Mi + 5Mi = 15Mi; init peak = max(1Mi, 1Mi + 5Mi) = 6Mi; + 1Mi overhead
    assert_eq!(d.memory_requests, 16 * 1024 * 1024);
}

#[test]
fn finished_unscheduled_and_foreign_objects_have_no_placement() {
    let spec = json!({"nodeName": "n1", "containers": [container(json!({"cpu": "1"}), json!({}))]});
    assert!(PodPlacement::of_pod(&pod(spec.clone(), "Succeeded")).is_none());
    assert!(PodPlacement::of_pod(&pod(spec.clone(), "Failed")).is_none());
    assert!(PodPlacement::of_pod(&pod(spec, "Unknown")).is_some());
    let unscheduled = pod(json!({"containers": []}), "Pending");
    assert!(PodPlacement::of_pod(&unscheduled).is_none());
    assert!(PodPlacement::of_pod(&node(json!({}), json!({}), json!({}))).is_none());
}

#[test]
fn demand_adds_and_subtracts_exactly_and_never_goes_negative() {
    let a = Demand {
        pods: 2,
        cpu_requests: 500,
        cpu_limits: 1000,
        memory_requests: 1 << 20,
        memory_limits: 2 << 20,
    };
    let b = Demand {
        pods: 1,
        cpu_requests: 200,
        cpu_limits: 300,
        memory_requests: 1 << 10,
        memory_limits: 1 << 11,
    };
    assert_eq!((a + b) - b, a);
    assert_eq!(a - a, Demand::ZERO);
    assert_eq!(b - a, Demand::ZERO, "saturates at zero");
    let mut running = Demand::ZERO;
    running += a;
    running -= a;
    assert!(running.is_empty());
}

// --- allocation --------------------------------------------------------------------------

#[test]
fn the_allocation_compares_requests_and_limits_with_allocatable() {
    let res = node(
        json!({}),
        json!({}),
        json!({"allocatable": {"cpu": "3920m", "memory": "15Gi", "pods": "110"}}),
    );
    let summary = NodeSummary::from_resource(&res).unwrap();
    let demand = Demand {
        pods: 11,
        cpu_requests: 980,
        cpu_limits: 7840,
        memory_requests: 3 << 30,
        memory_limits: 30 << 30,
    };
    let alloc = NodeAllocation::new(&summary, &demand);
    assert_eq!(alloc.cpu.allocatable, Some(3920));
    assert_eq!(alloc.cpu.request_percent(), Some(25));
    assert_eq!(alloc.cpu.limit_percent(), Some(200));
    assert_eq!(alloc.cpu.requests_text(), "980m (25%)");
    assert_eq!(alloc.cpu.limits_text(), "7840m (200%)");
    assert_eq!(alloc.cpu.allocatable_text(), "3920m");
    assert!((alloc.cpu.request_fill() - 0.25).abs() < 1e-6);
    assert_eq!(alloc.cpu.limit_fill(), 1.0, "the bar is clamped");
    assert!(!alloc.cpu.is_overcommitted());

    assert_eq!(alloc.memory.request_percent(), Some(20));
    assert_eq!(alloc.memory.requests_text(), "3 GiB (20%)");
    assert_eq!(alloc.memory.limit_percent(), Some(200));

    assert_eq!(alloc.pods.requests, 11);
    assert_eq!(alloc.pods.request_percent(), Some(10));
    assert_eq!(alloc.pods.limits, None);
    assert_eq!(alloc.pods.limits_text(), "");
    assert_eq!(alloc.lines().len(), 3);
}

#[test]
fn a_node_that_reports_no_allocatable_has_no_percentages() {
    let summary = NodeSummary::from_resource(&node(json!({}), json!({}), json!({}))).unwrap();
    let alloc = NodeAllocation::new(
        &summary,
        &Demand {
            pods: 1,
            cpu_requests: 100,
            ..Demand::ZERO
        },
    );
    assert_eq!(alloc.cpu.request_percent(), None);
    assert_eq!(alloc.cpu.requests_text(), "100m");
    assert_eq!(alloc.cpu.allocatable_text(), "-");
    assert_eq!(alloc.cpu.request_fill(), 0.0);
}

#[test]
fn requests_over_allocatable_are_flagged() {
    let res = node(
        json!({}),
        json!({}),
        json!({"allocatable": {"cpu": "1", "pods": "1"}}),
    );
    let summary = NodeSummary::from_resource(&res).unwrap();
    let alloc = NodeAllocation::new(
        &summary,
        &Demand {
            pods: 2,
            cpu_requests: 1500,
            ..Demand::ZERO
        },
    );
    assert!(alloc.cpu.is_overcommitted());
    assert!(alloc.pods.is_overcommitted());
    assert_eq!(alloc.cpu.request_percent(), Some(150));
}
