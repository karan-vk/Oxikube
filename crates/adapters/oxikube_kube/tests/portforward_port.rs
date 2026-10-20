//! Kind integration for E15-S01: `PortForwardPort::start`, the port-level forward with its
//! stats, error stream and typed error taxonomy. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::net::{IpAddr, Ipv4Addr};

use futures::StreamExt;
use oxikube_kube::KubePortForward;
use oxikube_ports::{PodRef, PortForwardError, PortForwardPort};
use oxikube_testkit::integration::TestNamespace;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use common::DEADLINE;
use common::portforward::{create_nginx_pod, get_welcome_page, wait_ready};

/// The port-level `start` (the inherent `KubePortForward::start` takes a `ForwardSpec`).
async fn start(
    forwarder: &KubePortForward,
    pod: PodRef,
    remote: u16,
    bind: IpAddr,
    local: Option<u16>,
) -> Result<oxikube_ports::ForwardHandle, PortForwardError> {
    PortForwardPort::start(forwarder, pod, remote, bind, local).await
}

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

#[tokio::test]
async fn start_forwards_to_a_pod_counts_traffic_and_stop_closes_the_listener() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;
    let forwarder = KubePortForward::new((*client).clone());

    let handle = start(
        &forwarder,
        PodRef::new(ns.name(), "web"),
        80,
        LOOPBACK,
        None,
    )
    .await
    .expect("start");
    assert!(handle.local_addr.ip().is_loopback());
    assert_ne!(
        handle.local_addr.port(),
        0,
        "None asks the OS for a free port"
    );

    let addr = handle.local_addr;
    let response = get_welcome_page(addr).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    common::wait_until("the counters settle", DEADLINE, || async {
        let s = handle.stats.snapshot();
        (s.total_connections == 1 && s.active_connections == 0 && s.bytes_in > 0 && s.bytes_out > 0)
            .then_some(())
    })
    .await;

    // A connection open across the stop sees its stream close.
    let mut open = tokio::net::TcpStream::connect(addr).await.expect("connect");
    open.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .expect("write");
    handle.stop();
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(DEADLINE, open.read_to_end(&mut rest)).await;
    assert!(closed.is_ok(), "the open connection ends after stop");
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "listener closed"
    );
}

#[tokio::test]
async fn start_reports_a_missing_pod_a_busy_port_and_a_closed_remote_port() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;
    let forwarder = KubePortForward::new((*client).clone());

    // Pod gone.
    let err = start(
        &forwarder,
        PodRef::new(ns.name(), "missing"),
        80,
        LOOPBACK,
        None,
    )
    .await
    .expect_err("no such pod");
    assert!(matches!(err, PortForwardError::PodGone(_)), "{err:?}");

    // Bind failed: the port is taken, and the error suggests another.
    let first = start(
        &forwarder,
        PodRef::new(ns.name(), "web"),
        80,
        LOOPBACK,
        None,
    )
    .await
    .expect("start");
    let busy = first.local_addr.port();
    let err = start(
        &forwarder,
        PodRef::new(ns.name(), "web"),
        80,
        LOOPBACK,
        Some(busy),
    )
    .await
    .expect_err("port in use");
    assert!(
        matches!(
            err,
            PortForwardError::BindFailed {
                kind: std::io::ErrorKind::AddrInUse,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.suggestion().is_some());

    // Port closed: nothing listens on 81 in the pod; the refusal arrives on `errors`.
    let mut closed = start(
        &forwarder,
        PodRef::new(ns.name(), "web"),
        81,
        LOOPBACK,
        None,
    )
    .await
    .expect("start: the pod cannot tell yet");
    let mut socket = tokio::net::TcpStream::connect(closed.local_addr)
        .await
        .expect("connect");
    let _ = socket.write_all(b"ping").await;
    let reported = tokio::time::timeout(DEADLINE, closed.errors.next())
        .await
        .expect("an error is reported")
        .expect("stream open");
    assert!(
        matches!(reported, PortForwardError::PortClosed { port: 81, .. }),
        "{reported:?}"
    );
}
