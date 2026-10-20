//! Error mapping and the kube-backed pieces that a scripted API server can answer.

use http::StatusCode;
use kube::client::UpgradeConnectionError;
use oxikube_domain::{ErrorKind, ForwardPort};
use oxikube_ports::{PodRef, PortForwardError, PortForwardPort};
use serde_json::json;

use crate::fake_api::{FakeApi, status_body};
use crate::remote::portforward::KubePortForward;
use crate::remote::portforward::cluster::{Cluster, KubeCluster};
use crate::remote::portforward::error::{bind_error, open_error, remote_failure, to_port_error};
use crate::remote::portforward::plan::{Plan, PodSelector};

fn upgrade_refused(status: StatusCode) -> kube::Error {
    kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(status))
}

#[test]
fn the_status_of_a_refused_upgrade_decides_the_error_kind() {
    let kind = |code: u16| {
        open_error(
            &upgrade_refused(StatusCode::from_u16(code).expect("code")),
            "ns",
            "web",
            80,
        )
        .kind()
    };
    assert_eq!(kind(404), ErrorKind::NotFound);
    assert_eq!(kind(403), ErrorKind::Forbidden);
    assert_eq!(kind(401), ErrorKind::Auth);
    assert_eq!(kind(400), ErrorKind::Validation);
    assert_eq!(kind(504), ErrorKind::Timeout);
    assert_eq!(kind(503), ErrorKind::Network);
    let forbidden = open_error(&upgrade_refused(StatusCode::FORBIDDEN), "ns", "web", 80);
    assert!(forbidden.message().contains("pods/portforward"));
}

#[test]
fn a_pod_side_error_is_one_bounded_redacted_line() {
    let long = format!(
        "an error occurred forwarding 8080 -> 80: {}\nsecond line",
        "x".repeat(1000)
    );
    let err = remote_failure(&long);
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.message().len() < 400, "{}", err.message().len());
    assert!(!err.message().contains("second line"));

    let leaked = remote_failure("dial failed: Authorization: Bearer abcdef1234567890abcdef");
    assert!(
        !leaked.message().contains("abcdef1234567890abcdef"),
        "{}",
        leaked.message()
    );
}

#[test]
fn bind_failures_map_to_the_error_taxonomy() {
    use std::io::{Error, ErrorKind as Io};
    let addr = "127.0.0.1:8080".parse().expect("addr");
    let in_use = bind_error(addr, &Error::from(Io::AddrInUse));
    assert_eq!(in_use.kind(), ErrorKind::Conflict);
    assert!(in_use.message().contains("8080"));
    assert_eq!(
        bind_error(addr, &Error::from(Io::PermissionDenied)).kind(),
        ErrorKind::Validation
    );
    assert_eq!(
        bind_error(addr, &Error::from(Io::AddrNotAvailable)).kind(),
        ErrorKind::Validation
    );
    assert_eq!(
        bind_error(addr, &Error::from(Io::Other)).kind(),
        ErrorKind::Internal
    );
}

#[tokio::test]
async fn forward_maps_the_clusters_refusals_without_a_websocket() {
    let api = FakeApi::new();
    api.reply(
        "/api/v1/namespaces/default/pods/gone/portforward",
        404,
        status_body(404, "NotFound", "pods \"gone\" not found"),
    );
    api.reply(
        "/api/v1/namespaces/default/pods/locked/portforward",
        403,
        status_body(403, "Forbidden", "forbidden"),
    );
    let forwarder = KubePortForward::new(api.client());

    let err = forwarder
        .forward("default", "gone", 80)
        .await
        .map(|_| ())
        .expect_err("404");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let err = forwarder
        .forward("default", "locked", 80)
        .await
        .map(|_| ())
        .expect_err("403");
    assert_eq!(err.kind(), ErrorKind::Forbidden);

    let upgrade = api
        .requests()
        .into_iter()
        .find(|r| r.path.ends_with("/gone/portforward"))
        .expect("the request was made");
    assert_eq!(upgrade.query.trim_start_matches('&'), "ports=80");
}

#[tokio::test]
async fn the_kube_cluster_reads_services_and_selects_pods_server_side() {
    let api = FakeApi::new();
    api.reply(
        "/api/v1/namespaces/default/services/web",
        200,
        json!({"metadata": {"name": "web"}, "spec": {
            "selector": {"app": "web"}, "ports": [{"port": 80, "targetPort": "http"}]}}),
    );
    api.reply(
        "/api/v1/namespaces/default/pods",
        200,
        json!({"kind": "PodList", "apiVersion": "v1", "metadata": {}, "items": [
            {"metadata": {"name": "web-1"}, "status": {"phase": "Running"}},
            {"metadata": {}},
        ]}),
    );
    let cluster = KubeCluster::new(api.client());

    let service = cluster.service("default", "web").await.expect("service");
    let plan = Plan::service("default", "web", service, &ForwardPort::Number(80)).expect("plan");
    let pods = cluster
        .pods("default", &plan.selector())
        .await
        .expect("pods");
    assert_eq!(
        pods.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["web-1"]
    );

    let by_name = cluster
        .pods("default", &PodSelector::Name("web-1".into()))
        .await
        .expect("pods");
    assert_eq!(by_name.len(), 1);
    let queries: Vec<String> = api
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/pods"))
        .map(|r| r.query)
        .collect();
    assert!(
        queries[0].contains("labelSelector=app%3Dweb"),
        "{queries:?}"
    );
    assert!(
        queries[1].contains("fieldSelector=metadata.name%3Dweb-1"),
        "{queries:?}"
    );

    let err = cluster
        .service("default", "missing")
        .await
        .expect_err("no such service");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

/// The one mapping table: every native failure the issue names, to the typed taxonomy.
#[test]
fn native_failures_map_to_the_port_forward_taxonomy() {
    use std::io::{Error, ErrorKind as Io};
    let pod = PodRef::new("ns", "web");
    let addr = |port: u16| std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let map = |err: oxikube_domain::OxiError, port: u16| to_port_error(&err, &pod, addr(port), 80);

    // 404 on the websocket upgrade: the pod is gone.
    let err = open_error(&upgrade_refused(StatusCode::NOT_FOUND), "ns", "web", 80);
    assert_eq!(map(err, 8080), PortForwardError::PodGone("ns/web".into()));
    // 403: RBAC on pods/portforward.
    let err = open_error(&upgrade_refused(StatusCode::FORBIDDEN), "ns", "web", 80);
    let mapped = map(err, 8080);
    assert!(
        matches!(mapped, PortForwardError::Forbidden(_)),
        "{mapped:?}"
    );
    assert!(
        mapped
            .suggestion()
            .expect("hint")
            .contains("pods/portforward")
    );
    // 400/422: the pod cannot take the stream (not running).
    let err = open_error(&upgrade_refused(StatusCode::BAD_REQUEST), "ns", "web", 80);
    assert_eq!(
        map(err, 8080),
        PortForwardError::PodNotReady("ns/web".into())
    );
    // Container refused the connection (reported on the error channel).
    let err = remote_failure(
        "error forwarding port 80: dial tcp 127.0.0.1:80: connect: connection refused",
    );
    let PortForwardError::PortClosed { port, detail } = map(err, 8080) else {
        panic!("expected PortClosed");
    };
    assert_eq!(port, 80);
    assert!(detail.contains("connection refused"), "{detail}");
    // Address in use, and permission denied on port 80.
    let in_use = map(bind_error(addr(8080), &Error::from(Io::AddrInUse)), 8080);
    assert_eq!(
        in_use,
        PortForwardError::BindFailed {
            addr: addr(8080),
            kind: Io::AddrInUse
        }
    );
    assert!(in_use.suggestion().expect("hint").contains("another port"));
    let denied = map(bind_error(addr(80), &Error::from(Io::PermissionDenied)), 80);
    assert_eq!(
        denied,
        PortForwardError::BindFailed {
            addr: addr(80),
            kind: Io::PermissionDenied
        }
    );
    assert!(denied.suggestion().expect("hint").contains("1024"));
    let unlocal = map(bind_error(addr(80), &Error::from(Io::AddrNotAvailable)), 80);
    assert!(matches!(
        unlocal,
        PortForwardError::BindFailed {
            kind: Io::AddrNotAvailable,
            ..
        }
    ));
    // Transport trouble and timeouts are a disconnect; pod states from the plan map too.
    assert!(matches!(
        map(oxikube_domain::OxiError::network("reset"), 1),
        PortForwardError::Disconnected(_)
    ));
    assert!(matches!(
        map(oxikube_domain::OxiError::timeout("slow"), 1),
        PortForwardError::Disconnected(_)
    ));
    assert_eq!(
        map(
            oxikube_domain::OxiError::conflict("pod ns/web is terminating"),
            1
        ),
        PortForwardError::PodNotReady("ns/web".into())
    );
    assert!(matches!(
        map(oxikube_domain::OxiError::internal("boom"), 1),
        PortForwardError::Other(_)
    ));
}
