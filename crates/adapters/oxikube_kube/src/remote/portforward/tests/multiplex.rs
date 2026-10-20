//! E15-S02: many connections on one listener, counters, the connection cap and the
//! graceful stop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_domain::{ErrorKind, ForwardPort, ForwardStatus, OxiError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::fakes::{
    Behaviour, DEADLINE, FakeCluster, FakeConnector, echo_through, eventually, pod, spec, start,
    start_capped, wait_status,
};
use crate::remote::portforward::accept::STOP_GRACE;
use crate::remote::portforward::handle::ForwardHandle;

/// The cluster must outlive the forward: dropping it ends the pod watch, which stops it.
struct Forward {
    _cluster: Arc<FakeCluster>,
    connector: Arc<FakeConnector>,
    handle: ForwardHandle,
}

async fn pod_forward(script: Vec<Behaviour>) -> Forward {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new(script);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    Forward {
        _cluster: cluster,
        connector,
        handle,
    }
}

/// A connection that has proven it reached the pod (one byte echoed).
async fn open_echoing(handle: &ForwardHandle) -> TcpStream {
    let mut socket = TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    socket.write_all(b"x").await.expect("write");
    let mut byte = [0u8];
    tokio::time::timeout(DEADLINE, socket.read_exact(&mut byte))
        .await
        .expect("echo in time")
        .expect("echo");
    socket
}

/// Reads until the peer ends the stream; `true` for a clean end of stream (FIN).
async fn ends_cleanly(socket: &mut TcpStream) -> bool {
    let mut rest = Vec::new();
    match tokio::time::timeout(DEADLINE, socket.read_to_end(&mut rest)).await {
        Ok(Ok(_)) => true,
        Ok(Err(_)) => false,
        Err(_) => panic!("the connection stayed open"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_connections_each_get_their_own_stream_and_are_counted() {
    const CONNS: usize = 100;
    const PAYLOAD: usize = 16 * 1024;
    let Forward {
        _cluster: _keep,
        connector,
        handle,
    } = pod_forward(vec![]).await;
    let addr = handle.local_addr();

    let clients = (0..CONNS).map(|i| {
        tokio::spawn(async move {
            let payload = vec![(i % 251) as u8; PAYLOAD];
            let mut socket = TcpStream::connect(addr).await.expect("connect");
            let (mut read, mut write) = socket.split();
            let send = async {
                write.write_all(&payload).await.expect("write");
                write.shutdown().await.expect("half-close");
            };
            let mut back = Vec::with_capacity(PAYLOAD);
            let (_, received) = tokio::join!(send, read.read_to_end(&mut back));
            received.expect("read");
            assert_eq!(back, payload, "connection {i} got its own bytes back");
        })
    });
    for client in futures::future::join_all(clients).await {
        client.expect("client task");
    }

    assert_eq!(
        connector.calls().len(),
        CONNS,
        "one pod stream per connection"
    );
    let stats = handle.stats().clone();
    eventually("every connection to close", || {
        stats.snapshot().active_connections == 0
    })
    .await;
    let s = stats.snapshot();
    assert_eq!(s.total_connections, CONNS as u64);
    assert_eq!(s.bytes_out, (CONNS * PAYLOAD) as u64);
    assert_eq!(s.bytes_in, (CONNS * PAYLOAD) as u64);
}

#[tokio::test]
async fn stop_closes_in_flight_connections_cleanly_within_the_grace_period() {
    let Forward {
        _cluster: _keep,
        handle,
        ..
    } = pod_forward(vec![]).await;
    let addr = handle.local_addr();
    let stats = handle.stats().clone();
    let status = handle.watch_status();
    let mut open = Vec::new();
    for _ in 0..3 {
        open.push(open_echoing(&handle).await);
    }
    assert_eq!(stats.snapshot().active_connections, 3);

    let began = Instant::now();
    handle.stop().await;
    assert!(
        began.elapsed() < STOP_GRACE,
        "connections that close cleanly do not wait out the grace ({:?})",
        began.elapsed()
    );
    assert_eq!(*status.borrow(), ForwardStatus::Stopped);
    for socket in &mut open {
        assert!(ends_cleanly(socket).await, "a clean FIN, not a reset");
    }
    assert_eq!(stats.snapshot().active_connections, 0);
    TcpListener::bind(addr)
        .await
        .expect("the port is free again");
}

#[tokio::test]
async fn the_graceful_stop_behind_the_port_handle_drains_and_stops() {
    let Forward {
        _cluster: _keep,
        handle,
        ..
    } = pod_forward(vec![]).await;
    let mut socket = open_echoing(&handle).await;
    let stopped = handle.stopped();

    handle.graceful_stop().await;
    tokio::time::timeout(DEADLINE, stopped)
        .await
        .expect("stopped() resolves");
    assert_eq!(handle.status(), ForwardStatus::Stopped);
    assert!(handle.is_finished());
    assert!(ends_cleanly(&mut socket).await);
}

#[tokio::test]
async fn dropping_the_handle_aborts_every_connection_task() {
    let Forward {
        _cluster: _keep,
        handle,
        ..
    } = pod_forward(vec![]).await;
    let stats = handle.stats().clone();
    let mut sockets = Vec::new();
    for _ in 0..5 {
        sockets.push(open_echoing(&handle).await);
    }

    drop(handle);

    // Each bridge holds an `ActiveConnection` guard, so zero means every task is gone.
    eventually("the bridges to be aborted", || {
        stats.snapshot().active_connections == 0
    })
    .await;
    for socket in &mut sockets {
        let mut buf = [0u8; 1];
        let read = tokio::time::timeout(DEADLINE, socket.read(&mut buf))
            .await
            .expect("the socket is closed");
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    }
}

#[tokio::test]
async fn connections_beyond_the_cap_are_closed_and_reported_once() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start_capped(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
        2,
    )
    .await
    .expect("start");
    let first = open_echoing(&handle).await;
    let _second = open_echoing(&handle).await;
    let mut events = handle.events();

    for _ in 0..3 {
        let mut refused = TcpStream::connect(handle.local_addr())
            .await
            .expect("the kernel accepts");
        let mut buf = [0u8; 1];
        let read = tokio::time::timeout(DEADLINE, refused.read(&mut buf))
            .await
            .expect("closed at once");
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    }
    let status = wait_status(&handle, |s| matches!(s, ForwardStatus::Error { .. })).await;
    assert!(
        matches!(
            &status,
            ForwardStatus::Error { kind: ErrorKind::BudgetExceeded, message } if message.contains('2')
        ),
        "{status:?}"
    );
    let mut transitions = 0;
    while events.try_recv().is_ok() {
        transitions += 1;
    }
    assert_eq!(transitions, 1, "three refusals, one error transition");
    assert_eq!(
        connector.calls().len(),
        2,
        "refused connections never reach the pod"
    );

    // Freeing a slot lets the next connection through, which clears the error.
    drop(first);
    let stats = handle.stats().clone();
    eventually("a slot to free up", || {
        stats.snapshot().active_connections == 1
    })
    .await;
    assert_eq!(echo_through(&handle, b"again").await, b"again");
    wait_status(&handle, |s| matches!(s, ForwardStatus::Listening { .. })).await;
}

#[tokio::test]
async fn a_pod_that_hangs_up_mid_stream_ends_the_local_socket_cleanly() {
    let Forward {
        _cluster: _keep,
        handle,
        ..
    } = pod_forward(vec![Behaviour::SendThenClose(b"partial".to_vec())]).await;
    let mut socket = TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    let mut got = Vec::new();
    tokio::time::timeout(DEADLINE, socket.read_to_end(&mut got))
        .await
        .expect("closed")
        .expect("a clean end of stream");
    assert_eq!(got, b"partial");

    // The listener keeps serving.
    assert_eq!(echo_through(&handle, b"next").await, b"next");
}

#[tokio::test]
async fn a_failing_connection_does_not_touch_its_siblings() {
    let Forward {
        _cluster: _keep,
        handle,
        ..
    } = pod_forward(vec![
        Behaviour::Echo,
        Behaviour::Refuse(OxiError::network("stream reset")),
    ])
    .await;
    let mut healthy = open_echoing(&handle).await;
    let _failing = TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    wait_status(&handle, |s| matches!(s, ForwardStatus::Error { .. })).await;

    healthy.write_all(b"still here").await.expect("write");
    let mut back = [0u8; 10];
    tokio::time::timeout(DEADLINE, healthy.read_exact(&mut back))
        .await
        .expect("sibling still echoes")
        .expect("read");
    assert_eq!(&back, b"still here");
}

#[tokio::test]
async fn losing_the_pod_closes_its_connections_and_stops_the_forward() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut socket = open_echoing(&handle).await;
    let mut events = handle.events();

    cluster.push(vec![]);

    let began = Instant::now();
    assert!(ends_cleanly(&mut socket).await);
    assert!(began.elapsed() < STOP_GRACE + Duration::from_millis(500));
    tokio::time::timeout(DEADLINE, handle.stopped())
        .await
        .expect("stopped");
    assert!(matches!(
        events.recv().await,
        Ok(ForwardStatus::TargetGone { .. })
    ));
}
