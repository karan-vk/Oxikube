//! k8s-openapi to model conversions and the pod set folded from watch events.

use k8s_openapi::api::core::v1::{Pod, Service};
use kube::runtime::watcher::Event;
use serde_json::{Value, json};

use crate::remote::portforward::plan::TargetPort;
use crate::remote::portforward::pods::{PodSet, pod_info, service_info};

fn pod_from(value: Value) -> Pod {
    serde_json::from_value(value).expect("a pod")
}

fn running_pod(name: &str, ready: &str) -> Pod {
    pod_from(json!({
        "metadata": {"name": name, "creationTimestamp": "2026-01-02T03:04:05Z"},
        "spec": {"containers": [
            {"name": "app", "ports": [
                {"name": "http", "containerPort": 8080},
                {"containerPort": 53, "protocol": "UDP"},
                {"containerPort": 9090, "protocol": "TCP"},
            ]},
            {"name": "sidecar"},
        ]},
        "status": {"phase": "Running", "conditions": [
            {"type": "Initialized", "status": "True"},
            {"type": "Ready", "status": ready},
        ]},
    }))
}

#[test]
fn a_pod_converts_with_state_age_and_tcp_ports_only() {
    let info = pod_info(&running_pod("web", "True")).expect("named");
    assert_eq!(info.name, "web");
    assert!(info.running && info.ready && !info.terminating);
    assert_eq!(info.created, 1_767_323_045);
    let ports: Vec<_> = info
        .ports
        .iter()
        .map(|p| (p.name.as_deref(), p.number))
        .collect();
    assert_eq!(
        ports,
        [(Some("http"), 8080), (None, 9090)],
        "UDP is not forwardable"
    );
}

#[test]
fn readiness_and_deletion_are_read_from_the_pod() {
    assert!(!pod_info(&running_pod("web", "False")).expect("named").ready);

    let mut pod = running_pod("web", "True");
    pod.metadata.deletion_timestamp = Some(pod.metadata.creation_timestamp.clone().expect("set"));
    assert!(pod_info(&pod).expect("named").terminating);

    let pending = pod_from(json!({"metadata": {"name": "p"}, "status": {"phase": "Pending"}}));
    let info = pod_info(&pending).expect("named");
    assert!(!info.running && !info.ready && info.ports.is_empty());

    assert!(
        pod_info(&pod_from(json!({"metadata": {}}))).is_none(),
        "no name"
    );
}

#[test]
fn a_service_converts_selector_and_target_ports() {
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": "web"},
        "spec": {
            "selector": {"app": "web"},
            "ports": [
                {"name": "http", "port": 80, "targetPort": "web"},
                {"port": 443, "targetPort": 8443},
                {"port": 8080},
                {"port": 53, "protocol": "UDP"},
            ],
        },
    }))
    .expect("a service");
    let info = service_info(&service);
    assert_eq!(info.selector["app"], "web");
    let ports: Vec<_> = info
        .ports
        .iter()
        .map(|p| (p.name.as_deref(), p.port, p.target.clone()))
        .collect();
    assert_eq!(
        ports,
        [
            (Some("http"), 80, TargetPort::Name("web".into())),
            (None, 443, TargetPort::Number(8443)),
            (None, 8080, TargetPort::Number(8080)),
        ]
    );
}

#[test]
fn the_pod_set_snapshots_on_every_change_and_swaps_atomically_on_relist() {
    let mut set = PodSet::default();
    let names = |snapshot: Vec<_>| -> Vec<String> {
        snapshot
            .into_iter()
            .map(|p: crate::remote::portforward::plan::PodInfo| p.name)
            .collect()
    };

    assert_eq!(set.apply(Event::Init), None);
    assert_eq!(set.apply(Event::InitApply(running_pod("a", "True"))), None);
    assert_eq!(set.apply(Event::InitApply(running_pod("b", "True"))), None);
    assert_eq!(
        names(set.apply(Event::InitDone).expect("listed")),
        ["a", "b"]
    );

    assert_eq!(
        names(
            set.apply(Event::Apply(running_pod("c", "True")))
                .expect("added")
        ),
        ["a", "b", "c"]
    );
    assert_eq!(
        names(
            set.apply(Event::Delete(running_pod("a", "True")))
                .expect("deleted")
        ),
        ["b", "c"]
    );
    assert_eq!(
        set.apply(Event::Delete(running_pod("zzz", "True"))),
        None,
        "unknown pod"
    );

    // A relist replaces the set in one step: `b` and `c` vanish only at InitDone.
    assert_eq!(set.apply(Event::Init), None);
    assert_eq!(set.apply(Event::InitApply(running_pod("d", "True"))), None);
    assert_eq!(names(set.apply(Event::InitDone).expect("relisted")), ["d"]);
}
