//! Starting, serving and stopping a forward.

use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{ErrorKind, ForwardPort, ForwardSpec, ForwardStatus, OxiError};
use tokio::net::TcpListener;

use super::fakes::{
    Behaviour, DEADLINE, FakeCluster, FakeConnector, echo_through, pod, service, service_port,
    spec, start, wait_status,
};
use crate::remote::portforward::plan::TargetPort;

fn pod_forward() -> (std::sync::Arc<FakeCluster>, std::sync::Arc<FakeConnector>) {
    (
        FakeCluster::new(None, vec![pod("web-1", 1)]),
        FakeConnector::new([]),
    )
}

#[tokio::test]
async fn a_pod_forward_listens_on_loopback_and_bridges_bytes_to_the_pod_port() {
    let (cluster, connector) = pod_forward();
    let spec = spec("Pod", "web-1", ForwardPort::Number(80));
    let handle = start(&cluster, &connector, &spec).await.expect("start");

    assert!(
        handle.local_addr().ip().is_loopback(),
        "{}",
        handle.local_addr()
    );
    assert_ne!(handle.local_addr().port(), 0, "port 0 picks a free port");
    assert_eq!(
        handle.status(),
        ForwardStatus::Listening {
            local_addr: handle.local_addr(),
            pod: "web-1".into()
        }
    );

    assert_eq!(
        echo_through(&handle, b"GET / HTTP/1.1\r\n\r\n").await,
        b"GET / HTTP/1.1\r\n\r\n"
    );
    assert_eq!(
        connector.calls(),
        [("default".to_owned(), "web-1".to_owned(), 80)]
    );
}

#[tokio::test]
async fn every_connection_gets_its_own_pod_connection() {
    let (cluster, connector) = pod_forward();
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    for round in 0..3u8 {
        assert_eq!(echo_through(&handle, &[round; 4]).await, [round; 4]);
    }
    assert_eq!(connector.calls().len(), 3);
}

#[tokio::test]
async fn a_service_forward_dials_the_target_port_of_a_ready_pod() {
    let service = service(
        &[("app", "web")],
        vec![service_port(
            Some("web"),
            80,
            TargetPort::Name("http".into()),
        )],
    );
    let cluster = FakeCluster::new(Some(service), vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Service", "web", ForwardPort::Number(80)),
    )
    .await
    .expect("start");

    assert_eq!(echo_through(&handle, b"hi").await, b"hi");
    assert_eq!(connector.calls()[0].1, "web-1");
    assert_eq!(
        connector.calls()[0].2,
        8080,
        "service port 80 maps to the named port http"
    );
}

#[tokio::test]
async fn a_busy_local_port_is_a_conflict_that_names_the_port() {
    let (cluster, connector) = pod_forward();
    let squatter = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let busy = squatter.local_addr().expect("addr").port();

    let spec = spec("Pod", "web-1", ForwardPort::Number(80)).with_local_port(busy);
    let err = start(&cluster, &connector, &spec)
        .await
        .expect_err("port is taken");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(
        err.message().contains(&busy.to_string()),
        "{}",
        err.message()
    );
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn dropping_the_handle_closes_the_listener_and_publishes_stopped() {
    let (cluster, connector) = pod_forward();
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let addr = handle.local_addr();
    let mut status = handle.watch_status();
    // An open connection must not keep the forward alive.
    let held = tokio::net::TcpStream::connect(addr).await.expect("connect");

    drop(handle);

    tokio::time::timeout(DEADLINE, status.wait_for(ForwardStatus::is_terminal))
        .await
        .expect("stopped in time")
        .expect("status channel");
    // The port is free again once the task is gone.
    let rebound = tokio::time::timeout(DEADLINE, async {
        loop {
            match TcpListener::bind(addr).await {
                Ok(listener) => break listener,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("the listener closed");
    drop((held, rebound));
}

#[tokio::test]
async fn stop_waits_until_the_listener_is_closed() {
    let (cluster, connector) = pod_forward();
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let addr = handle.local_addr();
    let status = handle.watch_status();
    handle.stop().await;
    assert_eq!(*status.borrow(), ForwardStatus::Stopped);
    TcpListener::bind(addr)
        .await
        .expect("rebind right after stop");
}

#[tokio::test]
async fn a_connection_error_is_published_and_cleared_by_the_next_good_connection() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([
        Behaviour::ServerError(OxiError::network("connection refused")),
        Behaviour::Echo,
    ]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");

    let _first = tokio::net::TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    let status = wait_status(&handle, |s| matches!(s, ForwardStatus::Error { .. })).await;
    let ForwardStatus::Error { kind, message } = status else {
        unreachable!()
    };
    assert_eq!(kind, ErrorKind::Network);
    assert!(message.contains("connection refused"));

    assert_eq!(echo_through(&handle, b"ok").await, b"ok");
    wait_status(&handle, |s| matches!(s, ForwardStatus::Listening { .. })).await;
}

#[tokio::test]
async fn a_failure_to_open_the_pod_connection_is_published() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([Behaviour::Refuse(OxiError::forbidden(
        "no pods/portforward",
    ))]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let _socket = tokio::net::TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    let status = wait_status(&handle, |s| matches!(s, ForwardStatus::Error { .. })).await;
    assert!(matches!(
        status,
        ForwardStatus::Error {
            kind: ErrorKind::Forbidden,
            ..
        }
    ));
}

#[tokio::test]
async fn requests_that_cannot_be_served_are_refused_before_anything_listens() {
    let (cluster, connector) = pod_forward();

    // Missing pod.
    let err = start(
        &cluster,
        &connector,
        &spec("Pod", "gone", ForwardPort::Number(80)),
    )
    .await
    .map(|_| ())
    .expect_err("no such pod");
    assert_eq!(err.kind(), ErrorKind::NotFound);

    // A kind that cannot be forwarded to.
    let mut deployment = spec("Pod", "web-1", ForwardPort::Number(80));
    deployment.target = ResourceRef::new(
        ClusterId::new("test", &ContextName::from("ctx")),
        Gvk::new("apps", "v1", "Deployment"),
        Some("default".into()),
        "web",
    );
    let err = start(&cluster, &connector, &deployment)
        .await
        .map(|_| ())
        .expect_err("deployment");
    assert_eq!(err.kind(), ErrorKind::Validation);

    // No namespace.
    let mut cluster_scoped: ForwardSpec = spec("Pod", "web-1", ForwardPort::Number(80));
    cluster_scoped.target.namespace = None;
    let err = start(&cluster, &connector, &cluster_scoped)
        .await
        .map(|_| ())
        .expect_err("namespace");
    assert_eq!(err.kind(), ErrorKind::Validation);

    // A missing service.
    let err = start(
        &cluster,
        &connector,
        &spec("Service", "nope", ForwardPort::Number(80)),
    )
    .await
    .map(|_| ())
    .expect_err("no such service");
    assert_eq!(err.kind(), ErrorKind::NotFound);

    assert!(connector.calls().is_empty());
}

#[tokio::test]
async fn binding_another_address_is_honoured() {
    let (cluster, connector) = pod_forward();
    let spec =
        spec("Pod", "web-1", ForwardPort::Number(80)).with_bind("127.0.0.2".parse().expect("ip"));
    // 127.0.0.2 exists on Linux; elsewhere the bind fails with a validation error. Either way
    // the requested address is what is used, never silently loopback.
    match start(&cluster, &connector, &spec).await {
        Ok(handle) => assert_eq!(
            handle.local_addr().ip(),
            "127.0.0.2".parse::<std::net::IpAddr>().expect("ip")
        ),
        Err(err) => assert_eq!(err.kind(), ErrorKind::Validation),
    }
}
