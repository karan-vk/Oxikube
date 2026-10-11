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
    let rows = rows_of(&multi_container_pod());
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
            "http:8080/TCP, metrics:9090/TCP",
            "250m/–",
            "256Mi/512Mi"
        ]
    );
    assert_eq!(
        texts(&rows[1])[6..],
        ["off:off:on", "admin:15000/TCP", "50m/200m", "64Mi/128Mi"]
    );
    assert_eq!(texts(&rows[3])[8..], ["500m/–", "–/–"]);
    assert_eq!(texts(&rows[4])[7], "–", "no ports");

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
    assert!(rows_of(&rs).is_empty());
}

fn panel(d: &mut Detail, view: &Entity<DetailView>) -> Entity<ContainersPanel> {
    d.read(view, |v| v.kind_panel().cloned())
        .expect("the pod's panel")
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
