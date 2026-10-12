//! The node panel's allocation section (E13-S11): requests, limits and live usage per resource,
//! GPUs and ephemeral storage, the pods that set none, metrics absent as a visible state, and the
//! usage poll that stops with the panel.

use std::time::Duration;

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::allocation::{AllocationKind, Usage};
use oxikube_domain::Resource;
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{MetricsCall, node, pod};
use serde_json::json;

use super::{node_detail, node_ref, panel_of, pod_on, tick, worker};
use crate::detail::tests::fixture::Detail;
use crate::kinds::allocation::USAGE_INTERVAL;

fn sample(name: &str, cpu_nanocores: u64, memory_bytes: u64) -> MetricsSample {
    let ts: Timestamp = "2026-10-12T10:00:00Z".parse().unwrap();
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        ts,
        None,
        cpu_nanocores,
        memory_bytes,
    )
}

/// Worker-1 (4 CPU, 16Gi, 110 pods) with 100Gi of ephemeral storage and 4 GPUs, and the pods:
/// `p1` asks 250m / 256Mi (limits 500m / 512Mi) and 1Gi of ephemeral storage, `gpu` takes two
/// GPUs, `bare` sets nothing.
fn gpu_worker() -> Vec<Resource> {
    let mut json = node().name("worker-1").json();
    json["status"]["allocatable"]["ephemeral-storage"] = json!("100Gi");
    json["status"]["allocatable"]["nvidia.com/gpu"] = json!("4");
    let mut p1 = pod()
        .namespace("x")
        .name("p1")
        .node("worker-1")
        .running()
        .requests("250m", "256Mi")
        .limits("500m", "512Mi")
        .json();
    p1["spec"]["containers"][0]["resources"]["requests"]["ephemeral-storage"] = json!("1Gi");
    let mut gpu = pod()
        .namespace("x")
        .name("gpu")
        .node("worker-1")
        .running()
        .requests("1", "1Gi")
        .limits("1", "1Gi")
        .json();
    gpu["spec"]["containers"][0]["resources"]["requests"]["nvidia.com/gpu"] = json!("2");
    gpu["spec"]["containers"][0]["resources"]["limits"]["nvidia.com/gpu"] = json!("2");
    let mut bare = pod()
        .namespace("x")
        .name("bare")
        .node("worker-1")
        .running()
        .json();
    bare["spec"]["containers"][0]["resources"] = json!({});
    vec![
        Resource::from_json(json).unwrap(),
        Resource::from_json(p1).unwrap(),
        Resource::from_json(gpu).unwrap(),
        Resource::from_json(bare).unwrap(),
    ]
}

/// The usage line of `kind` as the panel built it.
fn usage_of(
    d: &mut Detail,
    panel: &gpui::Entity<super::super::NodePanel>,
    kind: &AllocationKind,
) -> Usage {
    d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .allocation_view()
            .and_then(|view| view.row(kind))
            .map(|row| row.usage.clone())
            .expect("the row is built")
    })
}

#[gpui::test]
fn usage_joins_requests_and_limits_in_the_bars(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, gpu_worker());
    d.f.ports()
        .metrics
        .script()
        .node_metrics
        .push_ok(MetricsOutcome::Available(vec![
            sample("elsewhere", 9_000_000_000, 1 << 30),
            sample("worker-1", 1_200_000_000, 3 << 30),
        ]));
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    for _ in 0..3 {
        tick(&mut d);
    }

    for selector in [
        "node-allocation-cpu-fill",
        "node-allocation-cpu-usage",
        "node-allocation-cpu-limits",
        "node-allocation-cpu-requests",
        "node-allocation-cpu-limits-text",
        "node-allocation-cpu-usage-text",
        "node-allocation-memory-usage",
        "node-allocation-pods-requests",
        "node-allocation-ephemeral",
        "node-allocation-gpu-nvidia.com/gpu",
    ] {
        assert!(d.shown(selector), "{selector} is drawn");
    }
    // 1200m of 4 CPUs is 30 % of the track.
    let bar = d.f.vcx.debug_bounds("node-allocation-cpu-bar").unwrap();
    let usage = d.f.vcx.debug_bounds("node-allocation-cpu-usage").unwrap();
    let share = f32::from(usage.size.width) / f32::from(bar.size.width);
    assert!(
        (share - 0.3).abs() < 0.02,
        "the usage bar is {share} of the track"
    );

    let text = d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        let view = panel.allocation_view().unwrap();
        let row = |k: AllocationKind| view.row(&k).unwrap().clone();
        (
            row(AllocationKind::Cpu).usage_text(),
            row(AllocationKind::Memory).usage_text(),
            row(AllocationKind::EphemeralStorage).requests_text(),
            row(AllocationKind::Gpu("nvidia.com/gpu".into())).requests_text(),
            row(AllocationKind::Cpu).unset_text(),
        )
    });
    // 3 GiB of 16 GiB is 18.75 %.
    assert_eq!(text.0.as_deref(), Some("1200m of 4000m (30%)"));
    assert_eq!(text.1.as_deref(), Some("3 GiB of 16 GiB (18%)"));
    assert_eq!(text.2, "1 GiB of 100 GiB (1%)");
    assert_eq!(text.3, "2 of 4 (50%)");
    // `bare` sets no request and no limit; `p1` and `gpu` set both.
    assert_eq!(
        text.4.as_deref(),
        Some("1 pod set no request, 1 set no limit")
    );
    assert!(
        d.shown("node-allocation-cpu-unset"),
        "the unset count is on screen"
    );
}

#[gpui::test]
fn without_metrics_the_requests_stay_and_the_usage_says_why(cx: &mut TestAppContext) {
    let mut d = node_detail(
        cx,
        [
            worker(),
            pod_on("p1", "worker-1", ("250m", "256Mi"), ("500m", "512Mi")),
        ],
    );
    d.f.ports()
        .metrics
        .script()
        .node_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    for _ in 0..3 {
        tick(&mut d);
    }
    let reason = "metrics unavailable: metrics-server is not installed in this cluster";
    assert_eq!(
        usage_of(&mut d, &panel, &AllocationKind::Cpu),
        Usage::Unavailable(reason.into())
    );
    assert!(
        d.shown("node-allocation-cpu-usage-text"),
        "the reason is on screen"
    );
    assert!(
        d.shown("node-allocation-cpu-fill"),
        "the requests are still drawn"
    );
    assert!(
        !d.shown("node-allocation-cpu-usage"),
        "there is no usage bar, not a zero one"
    );
    assert!(
        d.shown("node-allocation-cpu-limits"),
        "the limits tick is a spec, still drawn"
    );
    let requests = d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .allocation_view()
            .and_then(|v| v.row(&AllocationKind::Cpu).map(|r| r.requests_text()))
    });
    assert_eq!(requests.as_deref(), Some("250m of 4000m (6%)"));
}

#[gpui::test]
fn a_failed_call_is_a_visible_state_too(cx: &mut TestAppContext) {
    // Nothing scripted: the fake answers with an error, as a cluster that times out would.
    let mut d = node_detail(cx, [worker()]);
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    for _ in 0..3 {
        tick(&mut d);
    }
    let Usage::Unavailable(text) = usage_of(&mut d, &panel, &AllocationKind::Memory) else {
        panic!("a failed call is an absence");
    };
    assert!(
        text.starts_with("metrics unavailable: the request failed"),
        "{text}"
    );
}

#[gpui::test]
fn the_poll_repeats_and_stops_with_the_panel(cx: &mut TestAppContext) {
    let mut d = node_detail(cx, [worker()]);
    let metrics = d.f.ports().metrics;
    for _ in 0..4 {
        metrics
            .script()
            .node_metrics
            .push_ok(MetricsOutcome::Available(vec![sample(
                "worker-1",
                400_000_000,
                1 << 30,
            )]));
    }
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    for _ in 0..2 {
        tick(&mut d);
    }
    let calls = |metrics: &oxikube_testkit::FakeMetricsPort| {
        metrics
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, MetricsCall::NodeMetrics(_)))
            .count()
    };
    assert_eq!(calls(&metrics), 1, "asked once on opening");
    d.f.vcx
        .executor()
        .advance_clock(USAGE_INTERVAL + Duration::from_secs(1));
    d.settle();
    assert_eq!(calls(&metrics), 2, "and again after the interval");

    // Close the drawer and let go of the handles: nothing observes, so nothing is asked.
    let drawer = d.drawer().expect("the drawer is open");
    d.f.vcx
        .update(|_, cx| drawer.update(cx, |drawer, cx| drawer.close(cx)));
    drop((panel, view));
    d.settle();
    d.f.vcx.executor().advance_clock(USAGE_INTERVAL * 4);
    d.settle();
    assert_eq!(calls(&metrics), 2, "polling ended with the panel");
}

#[gpui::test]
fn the_gpu_list_follows_the_setting_without_a_restart(cx: &mut TestAppContext) {
    use gpui::UpdateGlobal as _;
    use oxikube_settings::SettingsStore;

    let mut json = node().name("worker-1").json();
    json["status"]["allocatable"]["example.com/fpga"] = json!("2");
    let mut d = node_detail(cx, [Resource::from_json(json).unwrap()]);
    d.f.vcx.update(|_, cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        cx.set_global(store);
    });
    let view = d.open(&node_ref("worker-1"));
    let panel = panel_of(&mut d, &view);
    tick(&mut d);
    let rows = |d: &mut Detail| {
        d.f.vcx.update(|_, cx| {
            panel
                .read(cx)
                .allocation_view()
                .map(|v| v.rows.iter().map(|r| r.kind.id()).collect::<Vec<_>>())
                .unwrap()
        })
    };
    // The default list names no `example.com/fpga`; the testkit's node reports ephemeral storage.
    assert_eq!(rows(&mut d), ["cpu", "memory", "pods", "ephemeral"]);

    d.f.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "metrics": { "gpu_vendors": ["example.com/fpga"] } }"#)
                .expect("valid settings");
        });
    });
    d.settle();
    assert_eq!(
        rows(&mut d),
        ["cpu", "memory", "pods", "ephemeral", "gpu-example.com/fpga"],
        "the next build counts the listed resource"
    );
    assert!(d.shown("node-allocation-gpu-example.com/fpga"));
}
