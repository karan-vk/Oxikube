//! E12-S02: what the pods table and the containers drill-down read besides the printer columns:
//! service account, controller, the pod's effective requests and limits, and each container's
//! resources, probes and ports.

use oxikube_domain::view::{ComputeResources, ContainerKind, ContainerPort, Probes};
use oxikube_domain::{ContainerSummary, PodSummary, Quantity, Resource};
use serde_json::{Value, json};

fn pod(spec: Value, owners: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop", "ownerReferences": owners},
        "spec": spec,
        "status": {"phase": "Running"}
    }))
    .expect("a pod")
}

fn q(s: &str) -> Quantity {
    Quantity::parse(s).expect("a quantity")
}

fn container(name: &str, requests: Value, limits: Value) -> Value {
    json!({"name": name, "image": "busybox", "resources": {"requests": requests, "limits": limits}})
}

fn summary(spec: Value) -> PodSummary {
    PodSummary::from_resource(&pod(spec, json!([]))).expect("a summary")
}

#[test]
fn service_account_and_controller() {
    let res = pod(
        json!({"serviceAccountName": "builder", "containers": []}),
        json!([
            {"apiVersion": "v1", "kind": "Node", "name": "n", "uid": "1"},
            {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d8c7", "uid": "2", "controller": true}
        ]),
    );
    let s = PodSummary::from_resource(&res).unwrap();
    assert_eq!(s.service_account.as_deref(), Some("builder"));
    let owner = s.controlled_by.expect("a controller");
    assert_eq!((&*owner.kind, &*owner.name), ("ReplicaSet", "web-5d8c7"));

    // The deprecated field, and no controller among the owners.
    let res = pod(
        json!({"serviceAccount": "legacy", "containers": []}),
        json!([{"apiVersion": "v1", "kind": "Node", "name": "n", "uid": "1"}]),
    );
    let s = PodSummary::from_resource(&res).unwrap();
    assert_eq!(s.service_account.as_deref(), Some("legacy"));
    assert!(s.controlled_by.is_none());
}

#[test]
fn pod_requests_sum_the_containers_with_real_quantities() {
    let s = summary(json!({"containers": [
        container("a", json!({"cpu": "250m", "memory": "64Mi"}), json!({"cpu": "1", "memory": "128Mi"})),
        container("b", json!({"cpu": "0.5", "memory": "1.5Gi"}), json!({"cpu": "1500m", "memory": "2Gi"})),
    ]}));
    let r = s.resources;
    assert_eq!(r.requests.cpu, Some(q("750m")));
    assert_eq!(r.requests.memory, Some(q("64Mi") + q("1.5Gi")));
    assert_eq!(r.limits.cpu, Some(q("2500m")));
    assert_eq!(r.limits.memory, Some(q("2176Mi")));
}

#[test]
fn small_and_numeric_quantities_parse() {
    let s = summary(json!({"containers": [
        container("a", json!({"cpu": "100000u", "memory": "1k"}), json!({})),
        container("b", json!({"cpu": "250000000n", "memory": 1000}), json!({})),
        container("c", json!({"cpu": 1}), json!({})),
    ]}));
    assert_eq!(s.resources.requests.cpu, Some(q("1350m")));
    assert_eq!(s.resources.requests.memory, Some(q("2k")));
}

#[test]
fn a_container_without_a_limit_makes_the_pod_unbounded() {
    let s = summary(json!({"containers": [
        container("a", json!({"cpu": "100m"}), json!({"cpu": "200m", "memory": "64Mi"})),
        container("b", json!({}), json!({"memory": "64Mi"})),
    ]}));
    let r = s.resources;
    assert_eq!(
        r.requests.cpu,
        Some(q("100m")),
        "a missing request counts as zero"
    );
    assert_eq!(r.requests.memory, None, "nobody requests memory");
    assert_eq!(r.limits.cpu, None, "b has no CPU limit");
    assert_eq!(r.limits.memory, Some(q("128Mi")));
}

#[test]
fn init_containers_and_sidecars_follow_the_scheduler() {
    // A large plain init container dominates; a sidecar adds to the regular containers.
    let s = summary(json!({
        "initContainers": [
            container("migrate", json!({"cpu": "2"}), json!({"cpu": "2"})),
            {"name": "proxy", "restartPolicy": "Always",
             "resources": {"requests": {"cpu": "100m"}, "limits": {"cpu": "100m"}}}
        ],
        "containers": [container("app", json!({"cpu": "500m"}), json!({"cpu": "1"}))]
    }));
    assert_eq!(s.resources.requests.cpu, Some(q("2")));
    assert_eq!(s.resources.limits.cpu, Some(q("2")));

    let s = summary(json!({
        "initContainers": [
            container("migrate", json!({"cpu": "100m"}), json!({})),
            {"name": "proxy", "restartPolicy": "Always",
             "resources": {"requests": {"cpu": "200m"}, "limits": {"cpu": "300m"}}}
        ],
        "containers": [container("app", json!({"cpu": "500m"}), json!({"cpu": "1"}))]
    }));
    assert_eq!(s.resources.requests.cpu, Some(q("700m")));
    assert_eq!(s.resources.limits.cpu, Some(q("1300m")));
}

#[test]
fn pod_level_resources_win() {
    let s = summary(json!({
        "resources": {"requests": {"cpu": "4"}, "limits": {"memory": "8Gi"}},
        "containers": [container("app", json!({"cpu": "500m", "memory": "1Gi"}), json!({"memory": "1Gi"}))]
    }));
    let r = s.resources;
    assert_eq!(r.requests.cpu, Some(q("4")));
    assert_eq!(r.requests.memory, Some(q("1Gi")));
    assert_eq!(r.limits.memory, Some(q("8Gi")));
}

#[test]
fn no_resources_at_all() {
    let s = summary(json!({"containers": [{"name": "app"}]}));
    assert_eq!(s.resources, ComputeResources::default());
}

#[test]
fn containers_carry_resources_probes_ports_and_a_type_marker() {
    let res = pod(
        json!({
            "initContainers": [
                {"name": "migrate", "image": "busybox"},
                {"name": "proxy", "image": "envoy", "restartPolicy": "Always",
                 "startupProbe": {"tcpSocket": {"port": 15000}}}
            ],
            "containers": [{
                "name": "app", "image": "registry.example/web:1.2",
                "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}},
                "readinessProbe": {"httpGet": {"path": "/ready", "port": "http"}},
                "ports": [
                    {"name": "http", "containerPort": 8080, "protocol": "TCP"},
                    {"containerPort": 9090},
                    {"name": "dns", "containerPort": 53, "protocol": "UDP"},
                    {"name": "bad", "containerPort": 700000}
                ],
                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"memory": "256Mi"}}
            }],
            "ephemeralContainers": [{"name": "debugger", "image": "busybox"}]
        }),
        json!([]),
    );
    let list = ContainerSummary::list_from_resource(&res).unwrap();
    let kinds: Vec<_> = list.iter().map(|c| (c.name.as_ref(), c.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("migrate", ContainerKind::Init),
            ("proxy", ContainerKind::Sidecar),
            ("app", ContainerKind::Regular),
            ("debugger", ContainerKind::Ephemeral)
        ]
    );
    let markers: Vec<_> = list.iter().map(|c| c.kind.marker()).collect();
    assert_eq!(markers, ["init", "sidecar", "", "ephemeral"]);

    let app = &list[2];
    assert_eq!(
        app.probes,
        Probes {
            liveness: true,
            readiness: true,
            startup: false
        }
    );
    assert_eq!(app.probes.display(), "on:on:off");
    assert_eq!(list[1].probes.display(), "off:off:on");
    assert_eq!(list[0].probes.display(), "off:off:off");

    let ports: Vec<String> = app.ports.iter().map(ContainerPort::to_string).collect();
    assert_eq!(ports, ["http:8080/TCP", "9090/TCP", "dns:53/UDP"]);
    assert_eq!(app.resources.requests.cpu, Some(q("100m")));
    assert_eq!(app.resources.requests.memory, Some(q("128Mi")));
    assert_eq!(app.resources.limits.cpu, None);
    assert_eq!(app.resources.limits.memory, Some(q("256Mi")));
    assert!(list[3].ports.is_empty());
}
