//! Kind integration for E04-S10: forward to an nginx pod and a service and GET through the
//! local listener, named ports, the pod's error channel, RBAC and a busy local port.
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::rbac::v1::PolicyRule;
use oxikube_domain::{ErrorKind, ForwardPort, ForwardStatus};
use oxikube_kube::KubePortForward;
use oxikube_ports::PortForwardPort;
use oxikube_testkit::integration::TestNamespace;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use common::portforward::{
    create_nginx_pod, create_nginx_service, get_welcome_page, spec, wait_ready,
};
use common::{DEADLINE, TestServiceAccount, wait_until};

#[tokio::test]
async fn forward_to_an_nginx_pod_and_get() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;

    let forwarder = KubePortForward::new((*client).clone());
    let handle = forwarder
        .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(80)))
        .await
        .expect("start the forward");

    assert!(
        handle.local_addr().ip().is_loopback(),
        "loopback by default"
    );
    assert_eq!(
        handle.status(),
        ForwardStatus::Listening {
            local_addr: handle.local_addr(),
            pod: "web".into()
        }
    );
    let response = get_welcome_page(handle.local_addr()).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

    // Several connections, each over its own websocket, at the same time.
    let addr = handle.local_addr();
    let gets = futures::future::join_all((0..5).map(|_| get_welcome_page(addr))).await;
    assert!(gets.iter().all(|r| r.contains("Welcome to nginx!")));

    // Dropping the handle frees the port.
    handle.stop().await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "listener closed"
    );
}

#[tokio::test]
async fn a_pod_forward_resolves_a_named_container_port() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;
    let forwarder = KubePortForward::new((*client).clone());

    let handle = forwarder
        .start(&spec(
            "Pod",
            ns.name(),
            "web",
            ForwardPort::Named("http".into()),
        ))
        .await
        .expect("start");
    get_welcome_page(handle.local_addr()).await;

    let err = forwarder
        .start(&spec(
            "Pod",
            ns.name(),
            "web",
            ForwardPort::Named("nope".into()),
        ))
        .await
        .expect_err("no such container port");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = forwarder
        .start(&spec("Pod", ns.name(), "missing", ForwardPort::Number(80)))
        .await
        .expect_err("no such pod");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn forward_through_a_service_maps_the_service_port_to_the_named_target_port() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web-1", "web").await;
    create_nginx_service(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web-1").await;
    let forwarder = KubePortForward::new((*client).clone());

    // By number and by name, same service port.
    for remote in [ForwardPort::Number(80), ForwardPort::Named("web".into())] {
        let handle = forwarder
            .start(&spec("Service", ns.name(), "web", remote))
            .await
            .expect("start");
        assert_eq!(
            handle.status(),
            ForwardStatus::Listening {
                local_addr: handle.local_addr(),
                pod: "web-1".into()
            }
        );
        let response = get_welcome_page(handle.local_addr()).await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    let err = forwarder
        .start(&spec(
            "Service",
            ns.name(),
            "web",
            ForwardPort::Number(8080),
        ))
        .await
        .expect_err("not a service port");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn a_service_without_a_ready_pod_is_refused_and_retryable() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_service(&client, ns.name(), "web", "web").await;
    let err = KubePortForward::new((*client).clone())
        .start(&spec("Service", ns.name(), "web", ForwardPort::Number(80)))
        .await
        .expect_err("nothing behind the service");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.is_retryable());
}

#[tokio::test]
async fn the_pods_error_channel_surfaces_as_a_status_error() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;
    let forwarder = KubePortForward::new((*client).clone());

    // Nothing listens on 81: the kubelet cannot connect and says so on the error channel.
    let handle = forwarder
        .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(81)))
        .await
        .expect("start: the pod does not know port 81 is closed");
    let mut socket = tokio::net::TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    let _ = socket.write_all(b"ping").await;
    let mut sink = Vec::new();
    let _ = tokio::time::timeout(DEADLINE, socket.read_to_end(&mut sink)).await;

    let mut status = handle.watch_status();
    let status = tokio::time::timeout(
        DEADLINE,
        status.wait_for(|s| matches!(s, ForwardStatus::Error { .. })),
    )
    .await
    .expect("an error is published")
    .expect("channel open")
    .clone();
    let ForwardStatus::Error {
        kind: error_kind,
        message,
    } = status
    else {
        unreachable!()
    };
    assert_eq!(error_kind, ErrorKind::Network);
    assert!(message.contains("81"), "names the port: {message}");

    // The forward itself keeps listening.
    assert!(!handle.is_finished());
}

#[tokio::test]
async fn without_the_portforward_permission_the_connection_is_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    create_nginx_pod(&admin, ns.name(), "web", "web").await;
    wait_ready(&admin, ns.name(), "web").await;

    let rules = vec![PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into(), "services".into()]),
        verbs: vec!["get".into(), "list".into(), "watch".into()],
        ..PolicyRule::default()
    }];
    let reader = TestServiceAccount::create(&admin, ns.name(), "reader", rules).await;
    let name = "forward-reader";
    let pool = kind.pool(kind.with_token_context(name, &reader.token));
    let client = pool.get(&name.into()).await.expect("reader client");
    let forwarder = KubePortForward::new((*client).clone());

    // The reads that resolve the target work; opening the stream does not.
    let handle = wait_until("the role is effective", DEADLINE, || async {
        forwarder
            .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(80)))
            .await
            .ok()
    })
    .await;
    drop(handle);
    let err = forwarder
        .forward(ns.name(), "web", 80)
        .await
        .map(|_| ())
        .expect_err("pods/portforward is not granted");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn a_busy_local_port_is_a_conflict_that_names_it() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;
    let forwarder = KubePortForward::new((*client).clone());

    let first = forwarder
        .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(80)))
        .await
        .expect("start");
    let busy = first.local_addr().port();
    let err = forwarder
        .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(80)).with_local_port(busy))
        .await
        .expect_err("the port is taken");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(
        err.message().contains(&busy.to_string()),
        "{}",
        err.message()
    );
}
