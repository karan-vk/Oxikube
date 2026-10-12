//! The node view: requests, limits and usage per resource against allocatable.

use oxikube_domain::utilisation::GpuVendors;

use super::{fixture, q};
use crate::allocation::{
    AllocationKind, AllocationView, BarBasis, MeasuredUsage, Usage, UsageState,
};
use crate::metrics::allocate_nodes;

fn view(usage: &UsageState) -> AllocationView {
    let (n1, pods) = fixture();
    let all = allocate_nodes([&n1], pods.iter(), &GpuVendors::default());
    AllocationView::for_node(&all[0], usage)
}

fn measured() -> UsageState {
    UsageState::Ready(MeasuredUsage {
        cpu: Ok(q("1200m")),
        memory: Ok(q("3Gi")),
        pods_reporting: 0,
        pods_unreadable: 0,
    })
}

#[test]
fn rows_carry_requests_limits_and_usage_against_allocatable() {
    let v = view(&measured());
    let ids: Vec<String> = v.rows.iter().map(|r| r.kind.id()).collect();
    assert_eq!(
        ids,
        ["cpu", "memory", "pods", "ephemeral", "gpu-nvidia.com/gpu"]
    );
    assert_eq!(v.pods, 5, "a, b, c, e and z: f finished, g is unplaced");

    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    // 500m + 250m + 1 + 1 of 4 CPUs: 2750m, 68.75 % rounded down; limits 1 + 3 + 1 = 5 CPUs.
    assert_eq!(cpu.requests_text(), "2750m of 4000m (68%)");
    assert_eq!(cpu.limits_text().unwrap(), "5000m of 4000m (125%)");
    assert_eq!(cpu.usage_text().unwrap(), "1200m of 4000m (30%)");
    let mem = v.row(&AllocationKind::Memory).unwrap();
    // 256Mi + 128Mi + 1Gi + 1Gi = 2432Mi = 2.375Gi of 8Gi (29.7 %); limits 512Mi + 2Gi + 1Gi.
    assert_eq!(mem.requests_text(), "2.4 GiB of 8 GiB (29%)");
    assert_eq!(mem.limits_text().unwrap(), "3.5 GiB of 8 GiB (43%)");
    assert_eq!(mem.usage_text().unwrap(), "3 GiB of 8 GiB (37%)");
    let pods = v.row(&AllocationKind::Pods).unwrap();
    assert_eq!(pods.requests_text(), "5 of 110 (4%)");
    assert_eq!(pods.limits_text(), None);
    assert_eq!(pods.usage_text(), None, "the pod count has no usage line");
    let eph = v.row(&AllocationKind::EphemeralStorage).unwrap();
    assert_eq!(eph.requests_text(), "1 GiB of 100 GiB (1%)");
    assert_eq!(eph.limits_text().unwrap(), "2 GiB of 100 GiB (2%)");
    assert_eq!(
        eph.usage_text().unwrap(),
        "usage is not reported by metrics-server"
    );
    let gpu = v
        .row(&AllocationKind::Gpu("nvidia.com/gpu".into()))
        .unwrap();
    assert_eq!(gpu.requests_text(), "2 of 4 (50%)");
    assert_eq!(gpu.limits_text().unwrap(), "2 of 4 (50%)");
}

#[test]
fn over_committed_limits_are_noted_not_an_error() {
    let v = view(&measured());
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert!(cpu.limits_overcommitted());
    assert!(
        !cpu.requests_exceed_capacity(),
        "requests fit: the scheduler saw to it"
    );
    assert_eq!(
        cpu.limits_note().unwrap(),
        "limits are over-committed (125%); this is normal"
    );
    let mem = v.row(&AllocationKind::Memory).unwrap();
    assert!(!mem.limits_overcommitted());
    assert_eq!(mem.limits_note(), None);
}

#[test]
fn the_bar_has_three_segments() {
    let v = view(&measured());
    let bar = v.row(&AllocationKind::Cpu).unwrap().bar().unwrap();
    assert_eq!(bar.basis, BarBasis::Requests);
    assert!((bar.fill - 0.6875).abs() < 1e-6);
    assert_eq!(
        bar.limits_mark,
        Some(1.0),
        "limits past the end pin at the end"
    );
    assert!((bar.usage.unwrap() - 0.3).abs() < 1e-6);
    assert!(!bar.full);
    let pods = v.row(&AllocationKind::Pods).unwrap().bar().unwrap();
    assert_eq!((pods.limits_mark, pods.usage), (None, None));
}

#[test]
fn pods_without_requests_or_limits_are_counted_per_resource() {
    let v = view(&measured());
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    // Requests: only z sets none. Limits: b and z.
    assert_eq!((cpu.unset.requests, cpu.unset.limits), (1, 2));
    assert_eq!(
        cpu.unset_text().unwrap(),
        "1 pod set no request, 2 set no limit"
    );
    let eph = v.row(&AllocationKind::EphemeralStorage).unwrap();
    assert_eq!(
        eph.unset_text().unwrap(),
        "4 pods set no request, 4 set no limit"
    );
    assert_eq!(v.row(&AllocationKind::Pods).unwrap().unset_text(), None);
}

#[test]
fn without_metrics_requests_and_limits_stay_and_usage_says_why() {
    let reason = "metrics unavailable: metrics-server is not installed in this cluster";
    let v = view(&UsageState::unavailable(
        oxikube_domain::metrics::MissingReason::NotInstalled,
    ));
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert_eq!(
        cpu.requests_text(),
        "2750m of 4000m (68%)",
        "from the specs: still there"
    );
    assert_eq!(cpu.usage, Usage::Unavailable(reason.into()));
    assert_eq!(cpu.usage_text().unwrap(), reason);
    assert_eq!(
        cpu.bar().unwrap().usage,
        None,
        "no usage segment, not a zero one"
    );
    assert_eq!(
        v.row(&AllocationKind::Memory)
            .unwrap()
            .usage_text()
            .unwrap(),
        reason
    );

    let pending = view(&UsageState::Pending);
    assert_eq!(
        pending
            .row(&AllocationKind::Cpu)
            .unwrap()
            .usage_text()
            .unwrap(),
        "reading usage..."
    );
}

#[test]
fn a_reading_missing_for_one_resource_is_unavailable_for_that_one_only() {
    use oxikube_domain::metrics::MissingReason;
    let v = view(&UsageState::Ready(MeasuredUsage {
        cpu: Ok(q("1")),
        memory: Err(MissingReason::NotYetAvailable),
        pods_reporting: 0,
        pods_unreadable: 0,
    }));
    assert!(matches!(
        v.row(&AllocationKind::Cpu).unwrap().usage,
        Usage::Value(_)
    ));
    let memory = v
        .row(&AllocationKind::Memory)
        .unwrap()
        .usage_text()
        .unwrap();
    assert!(
        memory.starts_with("metrics unavailable: no reading yet"),
        "{memory}"
    );
}

#[test]
fn a_node_without_gpus_or_ephemeral_storage_has_only_the_core_rows() {
    let n = super::node(
        "plain",
        serde_json::json!({"cpu": "2", "memory": "4Gi", "pods": "10"}),
    );
    let all = allocate_nodes([&n], [], &GpuVendors::default());
    let v = AllocationView::for_node(&all[0], &UsageState::Pending);
    let ids: Vec<String> = v.rows.iter().map(|r| r.kind.id()).collect();
    assert_eq!(ids, ["cpu", "memory", "pods"]);
    assert_eq!(
        v.row(&AllocationKind::Cpu).unwrap().requests_text(),
        "0 of 2000m (0%)"
    );
}

#[test]
fn a_gpu_vendor_that_is_not_configured_is_not_a_row() {
    let (n1, pods) = fixture();
    let all = allocate_nodes([&n1], pods.iter(), &GpuVendors::none());
    let v = AllocationView::for_node(&all[0], &UsageState::Pending);
    assert!(
        v.rows
            .iter()
            .all(|r| !matches!(r.kind, AllocationKind::Gpu(_)))
    );
}

#[test]
fn the_numbers_are_the_s02_sums() {
    let (n1, pods) = fixture();
    let all = allocate_nodes([&n1], pods.iter(), &GpuVendors::default());
    let v = AllocationView::for_node(&all[0], &UsageState::Pending);
    let s02 = &all[0];
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert_eq!(cpu.requests, s02.requests.cpu);
    assert_eq!(cpu.limits, Some(s02.limits.cpu));
    assert_eq!(cpu.capacity, Some(s02.node.allocatable.cpu));
    assert_eq!(cpu.requests_percent(), s02.requests_percent().cpu);
    assert_eq!(cpu.limits_percent(), s02.limits_percent().cpu);
    let mem = v.row(&AllocationKind::Memory).unwrap();
    assert_eq!(mem.requests_percent(), s02.requests_percent().memory);
    assert_eq!(mem.limits_percent(), s02.limits_percent().memory);
    // S02 divides in floating point, the row in exact quantities: equal to the last digit.
    let row = v
        .row(&AllocationKind::Pods)
        .unwrap()
        .requests_percent()
        .unwrap();
    let expected = s02.pods_percent().unwrap();
    assert!((row - expected).abs() < 1e-9, "{row} vs {expected}");
    assert_eq!(v.pods, s02.pods);
}
