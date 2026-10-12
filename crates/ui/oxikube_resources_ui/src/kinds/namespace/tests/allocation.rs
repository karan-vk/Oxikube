//! The Namespace panel's allocation section (E13-S11): the pods of the namespace summed against
//! its ResourceQuotas, live usage, the pods that set none, and every way the data can be missing.

use std::time::Duration;

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::allocate_namespaces;
use oxikube_app::allocation::{AllocationKind, Usage};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_domain::utilisation::GpuVendors;
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::pod;
use serde_json::json;

use super::{ns, ns_ref};
use crate::detail::tests::fixture::Detail;
use crate::kinds::namespace::panel::of;
use crate::table::tests::fixture::cluster;

fn shop_pod(name: &str, requests: (&str, &str), limits: Option<(&str, &str)>) -> Resource {
    let mut builder = pod()
        .namespace("shop")
        .name(name)
        .node("worker-1")
        .running()
        .requests(requests.0, requests.1);
    if let Some(limits) = limits {
        builder = builder.limits(limits.0, limits.1);
    }
    builder.build()
}

fn quota(name: &str, spec: serde_json::Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ResourceQuota",
        "metadata": {"name": name, "namespace": "shop", "resourceVersion": "1"},
        "spec": spec,
        "status": {"hard": spec["hard"].clone(), "used": {"requests.cpu": "1500m", "pods": "3"}},
    }))
    .unwrap()
}

/// `shop` holds three running pods, one that finished and one waiting for a node; the pods of
/// another namespace stay out.
///
/// CPU requests 250m + 750m + 100m (default) + 100m (unplaced) = 1200m; limits 500m + 1.
fn objects() -> Vec<Resource> {
    let mut done = pod()
        .namespace("shop")
        .name("done")
        .node("worker-1")
        .succeeded();
    done = done.requests("8", "8Gi");
    let mut waiting = pod().namespace("shop").name("waiting").build().to_value();
    waiting["status"]["phase"] = json!("Pending");
    vec![
        ns("shop", json!({})),
        shop_pod("a", ("250m", "256Mi"), Some(("500m", "512Mi"))),
        shop_pod("b", ("750m", "1Gi"), Some(("1", "2Gi"))),
        pod()
            .namespace("shop")
            .name("bare")
            .node("worker-1")
            .running()
            .build(),
        done.build(),
        Resource::from_json(waiting).unwrap(),
        pod()
            .namespace("other")
            .name("elsewhere")
            .node("worker-1")
            .running()
            .build(),
    ]
}

fn text(
    d: &mut Detail,
    panel: &gpui::Entity<super::super::panel::NamespacePanel>,
    kind: AllocationKind,
) -> Option<(String, Option<String>, Option<String>)> {
    d.f.vcx.update(|_, cx| {
        let row = panel.read(cx).allocation_view()?.row(&kind)?.clone();
        Some((row.requests_text(), row.limits_text(), row.usage_text()))
    })
}

fn open(d: &mut Detail) -> gpui::Entity<super::super::panel::NamespacePanel> {
    let view = d.open(&ns_ref("shop"));
    d.draw();
    let any = d.read(&view, |v| {
        v.kind_panel().cloned().expect("the namespace panel")
    });
    let panel = of(&any);
    for _ in 0..4 {
        d.f.vcx.executor().advance_clock(Duration::from_millis(300));
        d.settle();
    }
    panel
}

fn sample(cpu_nano: u64, memory: u64, name: &str) -> MetricsSample {
    let ts: Timestamp = "2026-10-12T10:00:00Z".parse().unwrap();
    let pod = oxikube_domain::ids::ResourceRef::namespaced(
        cluster(),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "shop",
        name,
    );
    MetricsSample::new(MetricsSubject::Pod { pod }, ts, None, cpu_nano, memory)
}

#[gpui::test]
fn the_aggregate_is_measured_against_the_quota(cx: &mut TestAppContext) {
    let mut d = Detail::new(
        cx,
        objects().into_iter().chain([quota(
            "compute",
            json!({"hard": {"requests.cpu": "4", "limits.cpu": "8", "requests.memory": "8Gi", "pods": "10"}}),
        )]),
    );
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.f.ports()
        .metrics
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(vec![
            sample(300_000_000, 200 << 20, "a"),
            sample(500_000_000, 700 << 20, "b"),
        ]));
    let panel = open(&mut d);

    for selector in [
        "namespace-allocation-heading",
        "namespace-allocation",
        "namespace-allocation-cpu",
        "namespace-allocation-cpu-bar",
        "namespace-allocation-cpu-fill",
        "namespace-allocation-cpu-usage",
        "namespace-allocation-cpu-requests",
        "namespace-allocation-cpu-limits-text",
        "namespace-allocation-cpu-usage-text",
        "namespace-allocation-cpu-unset",
        "namespace-allocation-memory",
        "namespace-allocation-pods",
        "namespace-quotas-heading",
        "namespace-quota-0",
        "namespace-quota-0-0",
    ] {
        assert!(d.shown(selector), "{selector} is drawn");
    }
    assert!(
        !d.shown("namespace-allocation-versus"),
        "there is a quota: no \"totals only\" note"
    );

    // Requests: 250m + 750m + 100m (the default of `bare`) + 100m (`waiting`, not yet placed);
    // `done` finished and holds nothing. Of the quota's 4 CPUs: 1200m is 30 %.
    let (requests, limits, usage) = text(&mut d, &panel, AllocationKind::Cpu).unwrap();
    assert_eq!(requests, "1200m of 4000m (30%)");
    assert_eq!(limits.as_deref(), Some("1500m of 8000m (18%)"));
    assert_eq!(usage.as_deref(), Some("800m of 4000m (20%)"));
    let (pods, _, _) = text(&mut d, &panel, AllocationKind::Pods).unwrap();
    assert_eq!(pods, "4 of 10 (40%)", "a, b, bare and waiting");

    // The displayed numbers are the S02 sums for the same objects.
    let all = objects();
    let sums = allocate_namespaces(all.iter(), &GpuVendors::default());
    let shop = sums.iter().find(|a| &*a.namespace == "shop").unwrap();
    let (cpu, mem) = d.f.vcx.update(|_, cx| {
        let view = panel.read(cx).allocation_view().unwrap().clone();
        (
            view.row(&AllocationKind::Cpu).unwrap().clone(),
            view.row(&AllocationKind::Memory).unwrap().clone(),
        )
    });
    assert_eq!(
        (cpu.requests, mem.requests),
        (shop.requests.cpu, shop.requests.memory)
    );
    assert_eq!(cpu.limits, Some(shop.limits.cpu));

    // The quota as the server counts it: used against hard.
    d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.quota_note(), None);
        assert_eq!(panel.allocation_note(), None);
    });
}

#[gpui::test]
fn without_a_quota_the_totals_stand_alone(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, objects());
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let panel = open(&mut d);
    let (requests, limits, _) = text(&mut d, &panel, AllocationKind::Cpu).unwrap();
    assert_eq!(
        (requests.as_str(), limits.as_deref()),
        ("1200m", Some("1500m"))
    );
    assert!(d.shown("namespace-allocation-cpu-requests"));
    assert!(
        !d.shown("namespace-allocation-cpu-bar"),
        "nothing to fill against"
    );
    assert!(!d.shown("namespace-quotas-heading"), "no quotas to list");
    assert!(d.shown("namespace-allocation-versus"), "and it says so");
}

#[gpui::test]
fn without_metrics_the_totals_stay_and_usage_says_why(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, objects());
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.f.ports()
        .metrics
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::Forbidden));
    let panel = open(&mut d);
    let reason = "metrics unavailable: you are not allowed to read the metrics API";
    let usage = d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .allocation_view()
            .and_then(|v| v.row(&AllocationKind::Cpu).map(|r| r.usage.clone()))
    });
    assert_eq!(usage, Some(Usage::Unavailable(reason.into())));
    assert!(d.shown("namespace-allocation-cpu-usage-text"));
    let (requests, _, _) = text(&mut d, &panel, AllocationKind::Cpu).unwrap();
    assert_eq!(requests, "1200m", "requests are specs: still there");
}

#[gpui::test]
fn pods_that_set_nothing_are_counted(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, objects());
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let panel = open(&mut d);
    let unset = d.f.vcx.update(|_, cx| {
        let view = panel.read(cx).allocation_view().unwrap().clone();
        view.row(&AllocationKind::Cpu).unwrap().unset_text()
    });
    // No limits on `bare` and `waiting` (the others set one); every pod has a request.
    assert_eq!(unset.as_deref(), Some("2 pods set no limit"));
}

#[gpui::test]
fn a_forbidden_pod_feed_is_said_not_shown_as_an_empty_namespace(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [ns("shop", json!({}))]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let resources = d.f.ports().resources;
    // The Namespace feed of the detail first, then the pods' feed refused.
    resources.script().watch.push_ok(
        oxikube_testkit::Timeline::immediate([oxikube_ports::DeltaBatch::from_deltas(vec![
            oxikube_ports::Delta::Restarted(vec![ns("shop", json!({}))]),
        ])])
        .keep_open(),
    );
    resources
        .script()
        .watch
        .push_err(OxiError::forbidden("pods is forbidden"));
    let panel = open(&mut d);
    d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        assert!(panel.allocation_view().is_none());
        assert_eq!(
            panel.allocation_note(),
            Some("You are not allowed to list the pods of this namespace.")
        );
    });
    assert!(d.shown("namespace-allocation-pending"));
}
