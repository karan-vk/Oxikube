//! The namespace view: aggregated requests, limits and usage against the quotas.

use serde_json::json;

use super::{fixture, q, quota};
use crate::allocation::{
    AllocationKind, AllocationView, BarBasis, MeasuredUsage, Usage, UsageState,
};
use crate::metrics::allocate_namespaces;
use oxikube_domain::utilisation::{GpuVendors, QuotaCaps};

fn shop(quotas: &[oxikube_domain::Resource], usage: &UsageState) -> AllocationView {
    let (_, pods) = fixture();
    let gpus = GpuVendors::default();
    let all = allocate_namespaces(pods.iter(), &gpus);
    let shop = all.iter().find(|a| &*a.namespace == "shop").unwrap();
    let (caps, scoped) = QuotaCaps::tightest(quotas, &gpus);
    AllocationView::for_namespace(shop, &caps, scoped, usage)
}

fn measured() -> UsageState {
    UsageState::Ready(MeasuredUsage {
        cpu: Ok(q("900m")),
        memory: Ok(q("1Gi")),
        pods_reporting: 3,
        pods_unreadable: 0,
    })
}

fn quotas() -> Vec<oxikube_domain::Resource> {
    vec![
        quota(
            "compute",
            json!({"hard": {"requests.cpu": "4", "limits.cpu": "8", "requests.memory": "8Gi", "pods": "20"}}),
            json!({}),
        ),
        quota(
            "best-effort",
            json!({"hard": {"pods": "1"}, "scopes": ["BestEffort"]}),
            json!({}),
        ),
    ]
}

#[test]
fn totals_are_measured_against_the_tightest_unscoped_quota() {
    let v = shop(&quotas(), &measured());
    assert_eq!(v.capacity_name(), "quota");
    assert_eq!(v.pods, 5, "a, b, c, z and the unplaced g: f finished");
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    // 500m + 250m + 1 + 100m = 1850m of 4 (46.25 %); limits 1 + 3 = 4 of 8 (50 %).
    assert_eq!(cpu.requests_text(), "1850m of 4000m (46%)");
    assert_eq!(cpu.limits_text().unwrap(), "4000m of 8000m (50%)");
    assert_eq!(cpu.usage_text().unwrap(), "900m of 4000m (22%)");
    let mem = v.row(&AllocationKind::Memory).unwrap();
    // 256Mi + 128Mi + 1Gi + 64Mi = 1472Mi of 8Gi (17.9 %); no memory limit cap.
    assert_eq!(mem.requests_text(), "1.4 GiB of 8 GiB (17%)");
    assert_eq!(
        mem.limits_text().unwrap(),
        "2.5 GiB",
        "no quota caps memory limits"
    );
    let pods = v.row(&AllocationKind::Pods).unwrap();
    assert_eq!(
        pods.requests_text(),
        "5 of 20 (25%)",
        "the scoped quota's 1 is not applied"
    );
    assert_eq!(v.notes.len(), 1, "{:?}", v.notes);
    assert!(
        v.notes[0].starts_with("1 scoped quota caps only some pods"),
        "{:?}",
        v.notes
    );
}

#[test]
fn the_bar_follows_the_requests_cap() {
    let v = shop(&quotas(), &measured());
    let bar = v.row(&AllocationKind::Cpu).unwrap().bar().unwrap();
    assert_eq!(bar.basis, BarBasis::Requests);
    assert!((bar.fill - 0.4625).abs() < 1e-6);
    assert_eq!(
        bar.limits_mark, None,
        "limits have their own cap: a mark on the requests scale would mislead"
    );
    assert_eq!(
        v.row(&AllocationKind::Cpu).unwrap().limits_note(),
        None,
        "4 CPUs of limits under an 8 CPU cap"
    );
    assert!((bar.usage.unwrap() - 0.225).abs() < 1e-6);
}

#[test]
fn without_quotas_there_are_totals_and_no_bar() {
    let v = shop(&[], &measured());
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert_eq!(cpu.requests_text(), "1850m");
    assert_eq!(cpu.limits_text().unwrap(), "4000m");
    assert_eq!(cpu.usage_text().unwrap(), "900m");
    assert!(cpu.bar().is_none());
    assert_eq!(cpu.requests_percent(), None);
    assert_eq!(v.row(&AllocationKind::Pods).unwrap().requests_text(), "5");
    assert!(v.notes.is_empty());
}

#[test]
fn a_quota_that_caps_only_limits_draws_the_limits() {
    let only_limits = quota("l", json!({"hard": {"limits.cpu": "8"}}), json!({}));
    let v = shop(&[only_limits], &measured());
    let bar = v.row(&AllocationKind::Cpu).unwrap().bar().unwrap();
    assert_eq!(bar.basis, BarBasis::Limits);
    assert!((bar.fill - 0.5).abs() < 1e-6, "4 of 8");
}

#[test]
fn quota_for_a_resource_no_pod_uses_still_gets_a_row() {
    let storage = quota(
        "s",
        json!({"hard": {"requests.ephemeral-storage": "10Gi", "requests.nvidia.com/gpu": "2"}}),
        json!({}),
    );
    let v = shop(&[storage], &measured());
    let eph = v.row(&AllocationKind::EphemeralStorage).unwrap();
    assert_eq!(eph.requests_text(), "1 GiB of 10 GiB (10%)");
    let gpu = v
        .row(&AllocationKind::Gpu("nvidia.com/gpu".into()))
        .unwrap();
    assert_eq!(gpu.requests_text(), "0 of 2 (0%)");
}

#[test]
fn without_metrics_totals_stay_and_usage_says_why() {
    let reason = "metrics unavailable: you are not allowed to read the metrics API";
    let v = shop(
        &quotas(),
        &UsageState::unavailable(oxikube_domain::metrics::MissingReason::Forbidden),
    );
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert_eq!(cpu.requests_text(), "1850m of 4000m (46%)");
    assert_eq!(cpu.usage, Usage::Unavailable(reason.into()));
}

#[test]
fn pods_with_no_reading_are_said_not_summed_as_zero() {
    let partial = UsageState::Ready(MeasuredUsage {
        cpu: Ok(q("900m")),
        memory: Ok(q("1Gi")),
        pods_reporting: 2,
        pods_unreadable: 2,
    });
    let v = shop(&[], &partial);
    assert!(
        v.notes
            .iter()
            .any(|n| n == "usage leaves out 2 pods with no reading yet")
    );

    // The namespace has pods but metrics-server has no sample for any of them yet.
    let none = UsageState::Ready(MeasuredUsage {
        cpu: Ok(q("0")),
        memory: Ok(q("0")),
        pods_reporting: 0,
        pods_unreadable: 0,
    });
    let v = shop(&[], &none);
    assert_eq!(
        v.row(&AllocationKind::Cpu).unwrap().usage_text().unwrap(),
        "metrics unavailable: no readings for the pods of this namespace yet"
    );
}

#[test]
fn the_numbers_are_the_s02_sums() {
    let (_, pods) = fixture();
    let gpus = GpuVendors::default();
    let all = allocate_namespaces(pods.iter(), &gpus);
    let s02 = all.iter().find(|a| &*a.namespace == "shop").unwrap();
    let (caps, scoped) = QuotaCaps::tightest(&quotas(), &gpus);
    let v = AllocationView::for_namespace(s02, &caps, scoped, &UsageState::Pending);
    let cpu = v.row(&AllocationKind::Cpu).unwrap();
    assert_eq!(cpu.requests, s02.requests.cpu);
    assert_eq!(cpu.limits, Some(s02.limits.cpu));
    let hard = caps.requests.amounts();
    assert_eq!(cpu.requests_percent(), s02.requests_vs(&hard).cpu);
    assert_eq!(
        v.row(&AllocationKind::Memory).unwrap().requests_percent(),
        s02.requests_vs(&hard).memory
    );
    assert_eq!(
        cpu.limits_percent(),
        s02.limits_vs(&caps.limits.amounts()).cpu
    );
    assert_eq!(v.pods, s02.pods);
}
