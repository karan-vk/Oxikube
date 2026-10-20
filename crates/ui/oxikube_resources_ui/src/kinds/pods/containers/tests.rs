//! The containers drill-down: the rows built from a pod (type markers, probes, ports,
//! requests/limits, state tones), and the panel at the top of a pod's detail, built once per
//! pod version.

use gpui::{Entity, TestAppContext};
use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use serde_json::json;

use super::{ContainerRow, ContainersPanel, rows_of};
use crate::detail::DetailView;
use crate::detail::tests::fixture::{Detail, pod_ref};

/// `shop/api-0`: a finished init container, a started sidecar, two regular containers (one
/// crash-looping) and a debug container.
pub(crate) fn multi_container_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "api-0", "namespace": "shop", "resourceVersion": "3",
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {
            "initContainers": [
                {"name": "migrate", "image": "registry.example/migrate:2"},
                {"name": "proxy", "image": "envoyproxy/envoy:v1.31", "restartPolicy": "Always",
                 "startupProbe": {"tcpSocket": {"port": 15000}},
                 "ports": [{"name": "admin", "containerPort": 15000}],
                 "resources": {"requests": {"cpu": "50m", "memory": "64Mi"},
                               "limits": {"cpu": "200m", "memory": "128Mi"}}}
            ],
            "containers": [
                {"name": "api", "image": "registry.example/api:1.4.2",
                 "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}},
                 "readinessProbe": {"httpGet": {"path": "/ready", "port": 8080}},
                 "ports": [{"name": "http", "containerPort": 8080, "protocol": "TCP"},
                           {"name": "metrics", "containerPort": 9090}],
                 "resources": {"requests": {"cpu": "250m", "memory": "256Mi"},
                               "limits": {"memory": "512Mi"}}},
                {"name": "worker", "image": "registry.example/worker:1.4.2",
                 "resources": {"requests": {"cpu": "0.5"}}}
            ],
            "ephemeralContainers": [{"name": "debugger", "image": "busybox:1.36"}]
        },
        "status": {
            "phase": "Running",
            "conditions": [{"type": "Initialized", "status": "True"}],
            "initContainerStatuses": [
                {"name": "migrate", "ready": false, "restartCount": 0,
                 "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
                {"name": "proxy", "ready": true, "started": true, "restartCount": 0,
                 "state": {"running": {"startedAt": "2026-01-01T00:00:05Z"}}}
            ],
            "containerStatuses": [
                {"name": "api", "ready": true, "started": true, "restartCount": 1,
                 "state": {"running": {"startedAt": "2026-01-01T00:00:06Z"}}},
                {"name": "worker", "ready": false, "started": false, "restartCount": 7,
                 "state": {"waiting": {"reason": "CrashLoopBackOff"}}}
            ],
            "ephemeralContainerStatuses": [
                {"name": "debugger", "ready": false, "restartCount": 0,
                 "state": {"running": {"startedAt": "2026-01-01T00:10:00Z"}}}
            ]
        }
    }))
    .expect("a pod")
}

fn texts(row: &ContainerRow) -> Vec<String> {
    row.cells().iter().map(ToString::to_string).collect()
}

#[test]
fn rows_carry_image_state_restarts_probes_ports_and_resources() {
    let rows = rows_of(&multi_container_pod(), None);
    let names: Vec<(&str, &str)> = rows.iter().map(|r| (&*r.name, r.marker)).collect();
    assert_eq!(
        names,
        [
            ("migrate", "init"),
            ("proxy", "sidecar"),
            ("api", ""),
            ("worker", ""),
            ("debugger", "ephemeral")
        ]
    );
    assert_eq!(
        texts(&rows[2]),
        [
            "2",
            "api",
            "registry.example/api:1.4.2",
            "true",
            "Running",
            "1",
            "on:on:off",
            "–",
            "–",
            "250m/–",
            "256Mi/512Mi",
            "–",
            "–",
            "–",
            "–",
            "http:8080/TCP, metrics:9090/TCP"
        ]
    );
    assert_eq!(
        texts(&rows[1])[6..11],
        ["off:off:on", "–", "–", "50m/200m", "64Mi/128Mi"]
    );
    assert_eq!(texts(&rows[1])[15], "admin:15000/TCP");
    assert_eq!(texts(&rows[3])[9..11], ["500m/–", "–/–"]);
    assert_eq!(texts(&rows[4])[15], "–", "no ports");

    let tones: Vec<Tone> = rows.iter().map(|r| r.state_tone).collect();
    assert_eq!(
        tones,
        [Tone::Ok, Tone::Ok, Tone::Ok, Tone::Error, Tone::Warn],
        "Completed, Running, Running, CrashLoopBackOff, running but not ready"
    );
}

#[test]
fn another_kind_has_no_rows() {
    let rs = oxikube_testkit::replicaset().build();
    assert!(rows_of(&rs, None).is_empty());
}

fn panel(d: &mut Detail, view: &Entity<DetailView>) -> Entity<ContainersPanel> {
    let pod_panel = d
        .read(view, |v| v.kind_panel().cloned())
        .expect("the pod's panel")
        .downcast::<crate::kinds::pods::PodPanel>()
        .expect("the pod panel");
    d.f.vcx
        .update(|_, cx| pod_panel.read(cx).containers().clone())
        .downcast::<ContainersPanel>()
        .expect("the containers panel")
}

#[gpui::test]
fn a_pods_detail_opens_on_its_containers(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [multi_container_pod()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&pod_ref("api-0"));

    assert!(d.shown("pod-containers"), "the sub-table is drawn");
    assert!(d.shown("pod-containers-heading"));
    assert!(d.shown("pod-containers-header-probes"));
    assert!(d.shown("pod-container-0-name"));
    assert!(d.shown("pod-container-3-state"));
    assert!(
        d.shown("detail-overview"),
        "the generic Overview stays below"
    );

    let panel = panel(&mut d, &view);
    let (count, builds) =
        d.f.vcx
            .update(|_, cx| (panel.read(cx).rows().len(), panel.read(cx).builds()));
    assert_eq!(count, 5);
    assert_eq!(builds, 1);

    // The detail redrawing (ages, focus, tabs) does not rebuild the rows: only a new version of
    // the pod does.
    for _ in 0..3 {
        d.update(&view, |_, cx| cx.notify());
        d.draw();
    }
    let builds = d.f.vcx.update(|_, cx| panel.read(cx).builds());
    assert_eq!(builds, 1);
}

/// `shop/api-0`'s reading: the sidecar and the two regular containers (metrics-server lists no
/// init container), the API at 90 % of its memory limit.
fn api_reading() -> oxikube_ports::MetricsOutcome {
    use oxikube_domain::metrics::{ContainerUsage, MetricsSample, MetricsSubject, Reading};
    let pod = oxikube_domain::ids::ResourceRef::namespaced(
        crate::table::tests::fixture::cluster(),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "shop",
        "api-0",
    );
    let at = "2026-01-01T00:20:00Z".parse().unwrap();
    let mut sample = MetricsSample::new(MetricsSubject::Pod { pod }, at, None, 0, 0);
    let mi = 1024 * 1024;
    let container = |name: &str, millicores: u64, mib: u64| ContainerUsage {
        name: name.into(),
        cpu: Reading::Value(millicores * 1_000_000),
        memory: Reading::Value(mib * mi),
    };
    sample.containers = vec![
        container("proxy", 20, 40),
        container("api", 180, 461),
        container("worker", 3, 12),
    ];
    oxikube_ports::MetricsOutcome::Available(vec![sample])
}

#[test]
fn rows_carry_each_containers_usage_and_its_share_of_request_and_limit() {
    let usage = oxikube_app::ClusterUsage::new();
    usage.apply(
        oxikube_app::MetricsKind::Pods,
        jiff::Timestamp::UNIX_EPOCH,
        &Ok(api_reading()),
    );
    let rows = rows_of(&multi_container_pod(), Some(&usage));
    // api: 180m of a 250m request, no CPU limit; 461Mi of 256Mi requested and 512Mi allowed.
    assert_eq!(texts(&rows[2])[7..9], ["180m", "461Mi"]);
    assert_eq!(texts(&rows[2])[11..15], ["72", "–", "180", "90"]);
    assert_eq!(rows[2].usage_tone("memory-limit-pct"), Some(Tone::Error));
    assert_eq!(
        rows[2].usage_tone("cpu-request-pct"),
        None,
        "requests are not toned"
    );
    // proxy (sidecar): 20m of 50m / 200m.
    assert_eq!(texts(&rows[1])[11..13], ["40", "10"]);
    assert_eq!(rows[1].usage_tone("cpu-limit-pct"), Some(Tone::Neutral));
    // The init container has no reading: `–`, never `0`.
    assert_eq!(texts(&rows[0])[7..9], ["–", "–"]);
}

#[gpui::test]
fn the_panel_watches_the_pods_and_shows_a_poll_within_a_second(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [multi_container_pod()]);
    let sessions = d.f.sessions.clone();
    let service = oxikube_app::MetricsService::new(
        sessions,
        std::sync::Arc::new(oxikube_testkit::FakeClockPort::default()),
    );
    d.f.vcx.update(|_, cx| {
        crate::kinds::init(cx);
        crate::metrics::set_service(cx, service.clone());
    });
    let view = d.open(&pod_ref("api-0"));
    let panel = panel(&mut d, &view);
    let cluster = crate::table::tests::fixture::cluster();
    let mut window = service.visibility();
    window.show(Some(cluster.clone()));
    let api_cpu = |d: &mut Detail| {
        d.f.vcx
            .update(|_, cx| panel.read(cx).rows()[2].cells()[7].to_string())
    };
    assert_eq!(api_cpu(&mut d), "–");
    assert_eq!(
        service.state(&cluster, oxikube_app::MetricsKind::Pods),
        oxikube_app::MetricsState::Loading,
        "the open panel watches the cluster's pods"
    );

    // A poll lands: the panel rebuilds on its next tick, once.
    let builds = d.f.vcx.update(|_, cx| panel.read(cx).builds());
    service.usage(&cluster).apply(
        oxikube_app::MetricsKind::Pods,
        jiff::Timestamp::UNIX_EPOCH,
        &Ok(api_reading()),
    );
    d.f.vcx.executor().advance_clock(super::USAGE_TICK);
    d.f.vcx.run_until_parked();
    assert_eq!(api_cpu(&mut d), "180m");
    d.f.vcx.executor().advance_clock(super::USAGE_TICK * 3);
    d.f.vcx.run_until_parked();
    let after = d.f.vcx.update(|_, cx| panel.read(cx).builds());
    assert_eq!(after, builds + 1, "a still second rebuilds nothing");
    assert!(d.shown("pod-container-2-memory-limit-pct"));
}

#[gpui::test]
fn a_hidden_detail_stops_watching_and_shows_a_missed_poll_when_shown_again(
    cx: &mut TestAppContext,
) {
    let mut d = Detail::new(cx, [multi_container_pod()]);
    let service = oxikube_app::MetricsService::new(
        d.f.sessions.clone(),
        std::sync::Arc::new(oxikube_testkit::FakeClockPort::default()),
    );
    d.f.vcx.update(|_, cx| {
        crate::kinds::init(cx);
        crate::metrics::set_service(cx, service.clone());
    });
    let view = d.open(&pod_ref("api-0"));
    let panel = panel(&mut d, &view);
    let cluster = crate::table::tests::fixture::cluster();
    let watching = |d: &mut Detail| d.f.vcx.update(|_, cx| panel.read(cx).is_watching());
    let builds = |d: &mut Detail| d.f.vcx.update(|_, cx| panel.read(cx).builds());
    assert!(watching(&mut d), "the shown detail watches the pods");

    // The drawer closes (or a pinned tab goes to the background): the view is kept, hidden.
    d.update(&view, |v, cx| v.set_shown(false, cx));
    d.f.vcx.run_until_parked();
    assert!(!watching(&mut d), "a hidden detail keeps no poller alive");

    // A poll lands while hidden: no tick runs, nothing is rebuilt.
    let before = builds(&mut d);
    service.usage(&cluster).apply(
        oxikube_app::MetricsKind::Pods,
        jiff::Timestamp::UNIX_EPOCH,
        &Ok(api_reading()),
    );
    d.f.vcx.executor().advance_clock(super::USAGE_TICK * 3);
    d.f.vcx.run_until_parked();
    assert_eq!(builds(&mut d), before, "the hidden panel has no tick");

    // Shown again: it watches again and shows the missed poll at once.
    d.update(&view, |v, cx| v.set_shown(true, cx));
    d.f.vcx.run_until_parked();
    assert!(watching(&mut d));
    let api_cpu =
        d.f.vcx
            .update(|_, cx| panel.read(cx).rows()[2].cells()[7].to_string());
    assert_eq!(api_cpu, "180m");

    // Another tab hides the panel too; the Overview shows it again.
    d.update(&view, |v, cx| v.set_tab(crate::detail::DetailTab::Yaml, cx));
    d.f.vcx.run_until_parked();
    assert!(!watching(&mut d), "the panel sits on the Overview only");
    d.update(&view, |v, cx| {
        v.set_tab(crate::detail::DetailTab::Overview, cx)
    });
    d.f.vcx.run_until_parked();
    assert!(watching(&mut d));
}

#[gpui::test]
fn a_forward_button_per_container_port_opens_the_dialog_on_that_port(cx: &mut TestAppContext) {
    let mut d = Detail::with_forward(cx, [multi_container_pod()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&pod_ref("api-0"));
    let panel = panel(&mut d, &view);
    let ports: Vec<u16> =
        d.f.vcx
            .update(|_, cx| panel.read(cx).ports().iter().map(|p| p.number).collect());
    assert_eq!(ports, [8080, 9090]);
    assert!(d.shown("pod-forward-8080") && d.shown("pod-forward-9090"));

    d.click("pod-forward-8080");
    let dialog = d
        .modal::<oxikube_portforward_ui::StartDialog>()
        .expect("Forward opens the start dialog");
    let input = d.f.vcx.update(|_, cx| dialog.read(cx).input(cx));
    assert_eq!(input.remote_port, "8080");
    assert!(
        d.shown("pf-port-0") && d.shown("pf-port-1"),
        "the pod's ports are offered"
    );
    assert!(d.f.forward.as_ref().unwrap().calls.lock().is_empty());
}

#[gpui::test]
fn without_a_port_forward_manager_there_are_no_forward_buttons(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [multi_container_pod()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.open(&pod_ref("api-0"));
    assert!(d.shown("pod-containers"));
    assert!(!d.shown("pod-forward-8080"));
}
