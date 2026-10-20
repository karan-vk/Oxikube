//! The ports offered come from the loaded object: TCP only, in the object's order.

use oxikube_testkit::resource;
use serde_json::json;

use crate::dialog::ports_of;

#[test]
fn a_pod_offers_the_tcp_ports_of_every_container() {
    let pod = resource("v1", "Pod")
        .name("web")
        .namespace("shop")
        .field(
            "spec",
            json!({"containers": [
                {"name": "app", "ports": [
                    {"name": "http", "containerPort": 8080},
                    {"containerPort": 53, "protocol": "UDP"},
                ]},
                {"name": "metrics", "ports": [{"containerPort": 9090, "protocol": "TCP"}]},
                {"name": "bare"},
            ]}),
        )
        .build();
    let ports = ports_of(&pod);
    let labels: Vec<_> = ports.iter().map(|p| p.label()).collect();
    assert_eq!(labels, ["http 8080 (app)", "9090 (metrics)"]);
}

#[test]
fn a_service_offers_its_tcp_ports() {
    let service = resource("v1", "Service")
        .name("api")
        .namespace("shop")
        .field(
            "spec",
            json!({"ports": [
                {"name": "web", "port": 80, "targetPort": "http"},
                {"name": "dns", "port": 53, "protocol": "UDP"},
            ]}),
        )
        .build();
    let ports = ports_of(&service);
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].number, 80);
    assert_eq!(ports[0].name.as_deref(), Some("web"));
    assert_eq!(ports[0].container, None);
}

#[test]
fn other_kinds_have_none() {
    let config = resource("v1", "ConfigMap")
        .name("c")
        .namespace("shop")
        .build();
    assert!(ports_of(&config).is_empty());
}
