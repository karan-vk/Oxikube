//! Service and pod resolution: a ready pod, the previous pod preferred, no ready endpoints,
//! named ports and several EndpointSlices.

use oxikube_domain::portforward::ForwardPort;
use serde_json::json;

use super::{pod, service, slice};
use crate::portforward::resolve::{Resolved, TargetView, Unresolved, resolve};

fn web_service() -> oxikube_domain::Resource {
    service(
        "web",
        json!([{"name": "http", "port": 80, "targetPort": "http"}, {"name": "metrics", "port": 9090}]),
    )
}

fn view(slices: Vec<oxikube_domain::Resource>) -> TargetView {
    TargetView {
        service: Some(Some(web_service())),
        slices: Some(slices),
        ..TargetView::default()
    }
}

fn http_ports() -> serde_json::Value {
    json!([{"name": "http", "port": 8080, "protocol": "TCP"}, {"name": "metrics", "port": 9090}])
}

fn resolved(pod: &str, port: u16) -> Result<Resolved, Unresolved> {
    Ok(Resolved {
        pod: pod.into(),
        port,
    })
}

#[test]
fn a_service_port_goes_to_a_ready_pod_on_its_target_port() {
    let v = view(vec![slice(
        "web-abc",
        "web",
        http_ports(),
        &[("web-b", true), ("web-a", false), ("web-c", true)],
    )]);
    assert_eq!(
        resolve(&v, true, &ForwardPort::Number(80), None),
        resolved("web-b", 8080),
        "the first ready pod by name, on the slice's resolved targetPort"
    );
    assert_eq!(
        resolve(&v, true, &ForwardPort::Named("metrics".into()), None),
        resolved("web-b", 9090),
        "a Service port by name"
    );
}

#[test]
fn the_previous_pod_is_kept_while_it_is_ready() {
    let v = view(vec![slice(
        "web-abc",
        "web",
        http_ports(),
        &[("web-a", true), ("web-c", true)],
    )]);
    assert_eq!(
        resolve(&v, true, &ForwardPort::Number(80), Some("web-c")),
        resolved("web-c", 8080)
    );
    assert_eq!(
        resolve(&v, true, &ForwardPort::Number(80), Some("web-gone")),
        resolved("web-a", 8080),
        "a previous pod that is not ready any more is replaced"
    );
}

#[test]
fn no_ready_endpoint_waits_and_says_so() {
    let v = view(vec![slice(
        "web-abc",
        "web",
        http_ports(),
        &[("web-a", false)],
    )]);
    let Err(Unresolved::Waiting(reason)) = resolve(&v, true, &ForwardPort::Number(80), None) else {
        panic!("waits for a ready pod");
    };
    assert!(reason.contains("no ready pod"), "{reason}");
    assert!(matches!(
        resolve(&view(vec![]), true, &ForwardPort::Number(80), None),
        Err(Unresolved::Waiting(_))
    ));
}

#[test]
fn several_slices_resolve_named_target_ports_per_pod() {
    // Pods that name `http` differently land in different slices, each with its own number.
    let v = view(vec![
        slice(
            "web-1",
            "web",
            json!([{"name": "http", "port": 8080}]),
            &[("web-b", false)],
        ),
        slice(
            "web-2",
            "web",
            json!([{"name": "http", "port": 3000}]),
            &[("web-a", true)],
        ),
    ]);
    assert_eq!(
        resolve(&v, true, &ForwardPort::Number(80), None),
        resolved("web-a", 3000)
    );
}

#[test]
fn a_port_the_service_lacks_is_invalid() {
    let v = view(vec![]);
    assert!(matches!(
        resolve(&v, true, &ForwardPort::Number(443), None),
        Err(Unresolved::Invalid(_))
    ));
    let udp = TargetView {
        service: Some(Some(service(
            "dns",
            json!([{"name": "dns", "port": 53, "protocol": "UDP"}]),
        ))),
        slices: Some(vec![]),
        ..TargetView::default()
    };
    assert!(matches!(
        resolve(&udp, true, &ForwardPort::Number(53), None),
        Err(Unresolved::Invalid(reason)) if reason.contains("only TCP")
    ));
    let missing = TargetView {
        service: Some(None),
        slices: Some(vec![]),
        ..TargetView::default()
    };
    assert!(matches!(
        resolve(&missing, true, &ForwardPort::Number(80), None),
        Err(Unresolved::Waiting(_))
    ));
}

#[test]
fn a_pod_target_must_be_running_and_names_its_ports() {
    let running = TargetView {
        pod: Some(Some(pod("api-0", "Running"))),
        ..TargetView::default()
    };
    assert_eq!(
        resolve(&running, false, &ForwardPort::Number(5432), None),
        resolved("api-0", 5432)
    );
    assert_eq!(
        resolve(&running, false, &ForwardPort::Named("http".into()), None),
        resolved("api-0", 8080),
        "a named port from the containers"
    );
    assert!(matches!(
        resolve(&running, false, &ForwardPort::Named("grpc".into()), None),
        Err(Unresolved::Invalid(_))
    ));
    let pending = TargetView {
        pod: Some(Some(pod("api-0", "Pending"))),
        ..TargetView::default()
    };
    assert!(matches!(
        resolve(&pending, false, &ForwardPort::Number(80), None),
        Err(Unresolved::Waiting(_))
    ));
    let gone = TargetView {
        pod: Some(None),
        ..TargetView::default()
    };
    assert!(matches!(
        resolve(&gone, false, &ForwardPort::Number(80), None),
        Err(Unresolved::Waiting(_))
    ));
    assert!(!TargetView::default().is_ready(false));
    assert!(running.is_ready(false));
}
