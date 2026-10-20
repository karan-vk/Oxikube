//! Scenario E: the error taxonomy as the manager surfaces it, with its suggestions.
//!
//! * a closed remote port: the forward binds (the pod cannot tell before a connection), the
//!   first connection is refused and the forward keeps `PortClosed`'s message as its last error;
//! * a missing pod: the session's port says `PodGone`; the manager keeps the forward waiting for
//!   the pod to exist (it never was `Active`) instead of failing it;
//! * a busy local port: the start fails with `BindFailed` (address in use) and the "try another
//!   port, or choose automatically" suggestion, and the forward is `Failed`.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use oxikube_app::portforward::{ForwardState, StartError};
use oxikube_domain::portforward::ForwardPort;
use oxikube_ports::{PodRef, PortForwardError};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::nginx;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{Forwarding, create, is_active};
use crate::cluster::Kind;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_port_missing_pod_and_busy_local_port() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let client = kind.admin_client().await;
    create::<Pod>(&client, ns.name(), nginx::pod("web", "web")).await;
    super::wait_ready_pod(&client, ns.name(), "web").await;
    let forwarding = Forwarding::connect(&kind).await;

    // Closed remote port.
    let closed = forwarding
        .manager
        .start_and_wait(forwarding.spec("Pod", ns.name(), "web", ForwardPort::Number(81)))
        .await
        .expect("binds: the pod cannot tell yet");
    let closed = forwarding.wait(closed.id, "active", is_active).await;
    let mut socket = tokio::net::TcpStream::connect(closed.local_addr.unwrap())
        .await
        .expect("the listener accepts");
    let _ = socket.write_all(b"GET / HTTP/1.1\r\n\r\n").await;
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut buf)).await;
    let closed = forwarding
        .wait(closed.id, "reporting the closed port", |i| {
            i.last_error.is_some()
        })
        .await;
    let message = closed.last_error.clone().unwrap();
    assert!(
        message.contains("nothing is listening on remote port 81"),
        "{message}"
    );
    assert!(
        is_active(&closed),
        "one refused connection does not stop the forward"
    );

    // Missing pod: the port's taxonomy, then the manager's view.
    let port = forwarding
        .sessions
        .get(&forwarding.cluster)
        .and_then(|s| s.port_forward())
        .expect("the session's port-forward port");
    let err = port
        .start(
            PodRef::new(ns.name(), "ghost"),
            80,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            None,
        )
        .await
        .expect_err("no such pod");
    assert!(matches!(err, PortForwardError::PodGone(_)), "{err:?}");
    assert!(err.suggestion().is_some(), "PodGone says what to do");
    let ghost = forwarding
        .manager
        .start_and_wait(forwarding.spec("Pod", ns.name(), "ghost", ForwardPort::Number(80)))
        .await
        .expect("waits for the pod rather than failing");
    let ghost = forwarding
        .wait(ghost.id, "waiting for the pod", |i| {
            matches!(i.state, ForwardState::Reconnecting { .. })
        })
        .await;
    assert!(
        ghost
            .state
            .reason()
            .is_some_and(|r| r.contains("does not exist")),
        "{:?}",
        ghost.state
    );

    // Busy local port.
    let taken = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("take a port");
    let busy = taken.local_addr().unwrap().port();
    let err = forwarding
        .manager
        .start_and_wait(
            forwarding
                .spec("Pod", ns.name(), "web", ForwardPort::Number(80))
                .with_local_port(busy),
        )
        .await
        .expect_err("the port is taken");
    let StartError::Forward(PortForwardError::BindFailed { addr, kind }) = &err else {
        panic!("not a bind failure: {err:?}");
    };
    assert_eq!((addr.port(), *kind), (busy, std::io::ErrorKind::AddrInUse));
    let hint = err.suggestion().expect("a suggestion");
    assert!(
        hint.contains(&busy.to_string()) && hint.contains("automatically"),
        "{hint}"
    );
    let failed = forwarding
        .manager
        .list()
        .into_iter()
        .find(|f| f.spec.local_port == busy)
        .expect("the failed forward stays listed");
    assert!(
        matches!(failed.state, ForwardState::Failed(_)),
        "{:?}",
        failed.state
    );
}
