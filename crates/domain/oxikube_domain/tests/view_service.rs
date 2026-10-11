//! Table-driven tests for `ServiceSummary`, `EndpointSliceSummary` and `EndpointsSummary`
//! (E12-S13): port text, external address, status, selectors, ready counts, named target ports.

use oxikube_domain::view::{
    EndpointSliceSummary, EndpointsSummary, NoSelector, ReadyCounts, SERVICE_NAME_LABEL,
    ServiceStatus, ServiceSummary, ServiceType, TargetPort, ready_counts, resolve_target_port,
};
use oxikube_domain::{Resource, ViewError};
use proptest::prelude::*;
use serde_json::{Value, json};

fn service(spec: Value, status: Value) -> ServiceSummary {
    let res = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": {"name": "web", "namespace": "demo"},
        "spec": spec, "status": status,
    }))
    .unwrap();
    ServiceSummary::from_resource(&res).unwrap()
}

#[test]
fn ports_are_formatted_like_kubectl() {
    struct Case {
        name: &'static str,
        spec: Value,
        ports: &'static str,
    }
    let cases = [
        Case {
            name: "cluster ip, protocol defaults to TCP",
            spec: json!({"type": "ClusterIP", "ports": [{"port": 80, "targetPort": 8080}]}),
            ports: "80/TCP",
        },
        Case {
            name: "node port",
            spec: json!({"type": "NodePort", "ports": [
                {"port": 80, "nodePort": 30080, "protocol": "TCP"},
                {"port": 53, "nodePort": 30053, "protocol": "UDP"},
            ]}),
            ports: "80:30080/TCP,53:30053/UDP",
        },
        Case {
            name: "load balancer",
            spec: json!({"type": "LoadBalancer", "ports": [{"port": 443, "nodePort": 31443}]}),
            ports: "443:31443/TCP",
        },
        Case {
            name: "external name has no ports",
            spec: json!({"type": "ExternalName", "externalName": "db.example.com"}),
            ports: "",
        },
        Case {
            name: "headless without ports",
            spec: json!({"clusterIP": "None", "selector": {"app": "db"}}),
            ports: "",
        },
        Case {
            name: "unnamed ports keep their order, a node port of zero is not shown",
            spec: json!({"ports": [{"port": 9000, "nodePort": 0}, {"port": 9001}]}),
            ports: "9000/TCP,9001/TCP",
        },
        Case {
            name: "an entry without a usable port is skipped",
            spec: json!({"ports": [{"name": "bad"}, {"port": 70000}, {"port": 81}]}),
            ports: "81/TCP",
        },
    ];
    for case in cases {
        assert_eq!(
            service(case.spec, json!({})).ports_text(),
            case.ports,
            "{}",
            case.name
        );
    }
}

#[test]
fn external_address_follows_the_service_type() {
    struct Case {
        name: &'static str,
        spec: Value,
        status: Value,
        external: &'static str,
        state: ServiceStatus,
    }
    let cases = [
        Case {
            name: "cluster ip shows its external ips",
            spec: json!({"externalIPs": ["1.2.3.4", "5.6.7.8"]}),
            status: json!({}),
            external: "1.2.3.4,5.6.7.8",
            state: ServiceStatus::Active,
        },
        Case {
            name: "cluster ip without any",
            spec: json!({}),
            status: json!({}),
            external: "",
            state: ServiceStatus::Active,
        },
        Case {
            name: "load balancer pending",
            spec: json!({"type": "LoadBalancer"}),
            status: json!({"loadBalancer": {}}),
            external: "<pending>",
            state: ServiceStatus::Pending,
        },
        Case {
            name: "load balancer with an ip and a hostname",
            spec: json!({"type": "LoadBalancer"}),
            status: json!({"loadBalancer": {"ingress": [{"ip": "34.1.2.3"}, {"hostname": "lb.example.com"}]}}),
            external: "34.1.2.3,lb.example.com",
            state: ServiceStatus::Assigned,
        },
        Case {
            name: "load balancer with an ip and external ips",
            spec: json!({"type": "LoadBalancer", "externalIPs": ["9.9.9.9"]}),
            status: json!({"loadBalancer": {"ingress": [{"ip": "34.1.2.3"}]}}),
            external: "34.1.2.3,9.9.9.9",
            state: ServiceStatus::Assigned,
        },
        Case {
            name: "load balancer pending with external ips still shows them",
            spec: json!({"type": "LoadBalancer", "externalIPs": ["9.9.9.9"]}),
            status: json!({}),
            external: "9.9.9.9",
            state: ServiceStatus::Pending,
        },
        Case {
            name: "external name",
            spec: json!({"type": "ExternalName", "externalName": "db.example.com"}),
            status: json!({}),
            external: "db.example.com",
            state: ServiceStatus::Active,
        },
        Case {
            name: "node port shows no load balancer state",
            spec: json!({"type": "NodePort"}),
            status: json!({"loadBalancer": {"ingress": [{"ip": "1.1.1.1"}]}}),
            external: "",
            state: ServiceStatus::Active,
        },
    ];
    for case in cases {
        let svc = service(case.spec, case.status);
        assert_eq!(svc.external_ip_text(), case.external, "{}", case.name);
        assert_eq!(svc.status, case.state, "{}", case.name);
    }
}

#[test]
fn cluster_ips_cover_headless_and_dual_stack() {
    let single = service(
        json!({"clusterIP": "10.0.0.1", "clusterIPs": ["10.0.0.1"]}),
        json!({}),
    );
    assert_eq!(single.cluster_ip_text(), "10.0.0.1");
    assert!(!single.headless);

    let dual = service(
        json!({
            "clusterIP": "10.0.0.1", "clusterIPs": ["10.0.0.1", "fd00::1"],
            "ipFamilies": ["IPv4", "IPv6"], "ipFamilyPolicy": "RequireDualStack"
        }),
        json!({}),
    );
    assert_eq!(
        dual.cluster_ip_text(),
        "10.0.0.1",
        "the column shows the primary"
    );
    assert_eq!(dual.cluster_ips.len(), 2);
    assert_eq!(dual.ip_families.len(), 2);

    let headless = service(
        json!({"clusterIP": "None", "clusterIPs": ["None"]}),
        json!({}),
    );
    assert!(headless.headless);
    assert_eq!(headless.cluster_ip_text(), "None");
    assert!(headless.cluster_ips.is_empty(), "None is not an address");

    let old = service(json!({"clusterIP": "10.0.0.9"}), json!({}));
    assert_eq!(old.cluster_ips.len(), 1, "falls back to clusterIP");
}

#[test]
fn selector_is_sorted_and_explains_when_missing() {
    let svc = service(
        json!({"selector": {"tier": "front", "app": "web"}}),
        json!({}),
    );
    assert_eq!(svc.selector_text(), "app=web,tier=front");
    assert_eq!(svc.pod_selector().as_deref(), Some("app=web,tier=front"));
    assert_eq!(svc.no_selector(), None);

    let manual = service(json!({"ports": [{"port": 80}]}), json!({}));
    assert_eq!(manual.pod_selector(), None);
    assert_eq!(manual.no_selector(), Some(NoSelector::Manual));
    assert!(NoSelector::Manual.explanation().contains("no selector"));

    // An ExternalName service ignores a selector it should not have.
    let external = service(
        json!({"type": "ExternalName", "externalName": "x.example.com", "selector": {"a": "b"}}),
        json!({}),
    );
    assert_eq!(external.pod_selector(), None);
    assert_eq!(external.no_selector(), Some(NoSelector::ExternalName));
    assert!(
        NoSelector::ExternalName
            .explanation()
            .contains("ExternalName")
    );
}

#[test]
fn publish_not_ready_and_session_affinity_are_read() {
    let svc = service(
        json!({"publishNotReadyAddresses": true, "sessionAffinity": "ClientIP"}),
        json!({}),
    );
    assert!(svc.publish_not_ready);
    assert_eq!(&*svc.session_affinity, "ClientIP");
    let plain = service(json!({}), json!({}));
    assert!(!plain.publish_not_ready);
    assert_eq!(&*plain.session_affinity, "None");
    assert_eq!(plain.kind, ServiceType::ClusterIP);
    assert_eq!(
        service(json!({"type": "Weird"}), json!({})).kind.as_str(),
        "Weird"
    );
}

#[test]
fn only_tcp_ports_open_in_a_browser_with_the_right_scheme() {
    let svc = service(
        json!({"ports": [
            {"name": "http", "port": 80},
            {"name": "https", "port": 8000},
            {"port": 443},
            {"name": "dns", "port": 53, "protocol": "UDP"},
            {"name": "web", "port": 9000, "appProtocol": "https"},
        ]}),
        json!({}),
    );
    let browsable: Vec<u16> = svc.browsable_ports().map(|p| p.port).collect();
    assert_eq!(browsable, [80, 8000, 443, 9000], "UDP is left out");
    let schemes: Vec<&str> = svc.browsable_ports().map(|p| p.browser_scheme()).collect();
    assert_eq!(schemes, ["http", "https", "https", "https"]);
    assert_eq!(svc.ports[0].browser_url(41234), "http://localhost:41234");
    assert_eq!(svc.ports[0].label(), "http (80)");
    assert_eq!(svc.ports[2].label(), "443");
}

fn pod_with_ports(containers: Value, init: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "p", "namespace": "demo"},
        "spec": {"containers": containers, "initContainers": init},
    }))
    .unwrap()
}

#[test]
fn named_target_ports_resolve_against_container_ports() {
    let svc = service(
        json!({"ports": [
            {"name": "http", "port": 80, "targetPort": "web"},
            {"name": "metrics", "port": 9090, "targetPort": "metrics", "protocol": "UDP"},
            {"name": "plain", "port": 81, "targetPort": 8081},
            {"name": "implicit", "port": 82},
        ]}),
        json!({}),
    );
    assert_eq!(svc.ports[0].target_port, TargetPort::Named("web".into()));
    assert_eq!(svc.ports[2].target_port, TargetPort::Number(8081));
    assert_eq!(
        svc.ports[3].target_port,
        TargetPort::Number(82),
        "defaults to port"
    );

    let none = pod_with_ports(
        json!([{"name": "a", "ports": [{"name": "other", "containerPort": 1}]}]),
        json!([]),
    );
    let sidecar = pod_with_ports(
        json!([{"name": "a"}, {"name": "b", "ports": [{"name": "web", "containerPort": 8080}]}]),
        json!([]),
    );
    let init = pod_with_ports(
        json!([]),
        json!([{"name": "i", "ports": [{"name": "web", "containerPort": 8181}]}]),
    );
    let udp = pod_with_ports(
        json!([{"name": "a", "ports": [{"name": "metrics", "containerPort": 9999, "protocol": "TCP"}]}]),
        json!([]),
    );
    assert_eq!(
        resolve_target_port(&svc.ports[0], [&none, &sidecar]),
        Some(8080),
        "first pod that names it"
    );
    assert_eq!(resolve_target_port(&svc.ports[0], [&none]), None);
    assert_eq!(resolve_target_port(&svc.ports[0], [&init]), Some(8181));
    assert_eq!(
        resolve_target_port(&svc.ports[1], [&udp]),
        None,
        "the protocol must match"
    );
    assert_eq!(
        resolve_target_port(&svc.ports[2], std::iter::empty()),
        Some(8081)
    );
}

fn slice(name: &str, endpoints: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice",
        "metadata": {"name": name, "namespace": "demo", "labels": {SERVICE_NAME_LABEL: "web"}},
        "addressType": "IPv4",
        "ports": [{"name": "http", "port": 8080, "protocol": "TCP"}, {"port": 9090}],
        "endpoints": endpoints,
    }))
    .unwrap()
}

#[test]
fn ready_counts_add_up_across_slices() {
    let a = EndpointSliceSummary::from_resource(&slice(
        "web-a",
        json!([
            {"addresses": ["10.0.0.1"], "conditions": {"ready": true}, "nodeName": "n1",
             "targetRef": {"kind": "Pod", "name": "web-1", "namespace": "demo"}},
            {"addresses": ["10.0.0.2"], "conditions": {"ready": false, "serving": true, "terminating": true}},
            {"addresses": ["10.0.0.3"]},
        ]),
    ))
    .unwrap();
    let b = EndpointSliceSummary::from_resource(&slice(
        "web-b",
        json!([{"addresses": ["10.0.1.1"], "conditions": {"ready": false}}]),
    ))
    .unwrap();
    let empty = EndpointSliceSummary::from_resource(&slice("web-c", json!([]))).unwrap();

    assert_eq!(
        a.ready_counts(),
        ReadyCounts {
            ready: 2,
            not_ready: 1
        },
        "absent ready counts as ready"
    );
    assert_eq!(
        ready_counts([&a, &b, &empty]),
        ReadyCounts {
            ready: 2,
            not_ready: 2
        }
    );
    assert_eq!(ready_counts([&a, &b]).total(), 4);
    assert_eq!(ready_counts(std::iter::empty()).summary(), "no endpoints");
    assert_eq!(
        ReadyCounts {
            ready: 3,
            not_ready: 0
        }
        .summary(),
        "3 ready"
    );
    assert_eq!(
        ReadyCounts {
            ready: 2,
            not_ready: 1
        }
        .summary(),
        "2 ready, 1 not ready"
    );
    assert_eq!(
        ReadyCounts {
            ready: 0,
            not_ready: 2
        }
        .summary(),
        "2 not ready"
    );

    assert_eq!(a.service.as_deref(), Some("web"));
    assert_eq!(&*a.address_type, "IPv4");
    assert_eq!(a.ports_text(), "8080/TCP,9090/TCP");
    let first = &a.endpoints[0];
    assert_eq!(first.node.as_deref(), Some("n1"));
    assert_eq!(first.target.as_ref().map(|t| &*t.name), Some("web-1"));
    let draining = &a.endpoints[1];
    assert!(!draining.ready && draining.serving && draining.terminating);
}

#[test]
fn endpoints_object_lists_ready_and_not_ready_addresses() {
    let res = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Endpoints",
        "metadata": {"name": "web", "namespace": "demo"},
        "subsets": [
            {"addresses": [{"ip": "10.0.0.1", "nodeName": "n1", "targetRef": {"kind": "Pod", "name": "web-1"}}],
             "notReadyAddresses": [{"ip": "10.0.0.2"}],
             "ports": [{"name": "http", "port": 8080, "protocol": "TCP"}]},
            {"addresses": [{"ip": "10.0.0.3"}], "ports": [{"name": "http", "port": 8080}]},
        ],
    }))
    .unwrap();
    let eps = EndpointsSummary::from_resource(&res).unwrap();
    assert_eq!(eps.addresses.len(), 3);
    assert_eq!(
        eps.ready_counts(),
        ReadyCounts {
            ready: 2,
            not_ready: 1
        }
    );
    assert_eq!(
        eps.ports.len(),
        1,
        "the same port in two subsets is listed once"
    );
    assert_eq!(
        eps.addresses[0].target.as_ref().map(|t| &*t.name),
        Some("web-1")
    );
}

#[test]
fn wrong_kind_is_the_only_error() {
    let pod = pod_with_ports(json!([]), json!([]));
    assert!(matches!(
        ServiceSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(matches!(
        EndpointSliceSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(matches!(
        EndpointsSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
}

proptest! {
    /// No JSON under `spec` / `status` / `endpoints` panics a constructor.
    #[test]
    fn arbitrary_shapes_never_panic(spec in arb_json(), status in arb_json(), endpoints in arb_json()) {
        let svc = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Service", "metadata": {"name": "s"},
            "spec": spec, "status": status,
        })).unwrap();
        if let Ok(s) = ServiceSummary::from_resource(&svc) {
            let _ = (s.ports_text(), s.external_ip_text(), s.selector_text(), s.cluster_ip_text());
        }
        let sl = Resource::from_json(json!({
            "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice", "metadata": {"name": "s"},
            "endpoints": endpoints, "ports": status,
        })).unwrap();
        let _ = EndpointSliceSummary::from_resource(&sl).map(|s| (s.ready_counts(), s.ports_text()));
    }
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-z]{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::from),
            prop::collection::hash_map("[a-zA-Z]{1,8}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}
