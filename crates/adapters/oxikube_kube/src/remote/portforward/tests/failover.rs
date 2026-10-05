//! The restart hook: a pod that goes away, and what each forward does about it.

use oxikube_domain::{ForwardPort, ForwardStatus};

use super::fakes::{
    DEADLINE, FakeCluster, FakeConnector, echo_through, pod, service, service_port, spec, start,
    wait_status,
};
use crate::remote::portforward::plan::TargetPort;

fn web_service() -> crate::remote::portforward::plan::ServiceInfo {
    service(
        &[("app", "web")],
        vec![service_port(None, 80, TargetPort::Number(8080))],
    )
}

async fn next_event(events: &mut tokio::sync::broadcast::Receiver<ForwardStatus>) -> ForwardStatus {
    tokio::time::timeout(DEADLINE, events.recv())
        .await
        .expect("an event arrives")
        .expect("channel is open")
}

#[tokio::test]
async fn a_service_forward_reports_target_gone_then_moves_to_another_pod() {
    let cluster = FakeCluster::new(Some(web_service()), vec![pod("web-1", 1), pod("web-2", 2)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Service", "web", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut events = handle.events();
    assert_eq!(echo_through(&handle, b"a").await, b"a");
    assert_eq!(connector.calls()[0].1, "web-1", "oldest ready pod first");

    // web-1 is deleted; web-2 is still there.
    cluster.push(vec![pod("web-2", 2)]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::TargetGone {
            pod: "web-1".into()
        }
    );
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::Listening {
            local_addr: handle.local_addr(),
            pod: "web-2".into()
        }
    );
    assert_eq!(echo_through(&handle, b"b").await, b"b");
    assert_eq!(connector.calls()[1].1, "web-2");
    assert!(!handle.is_finished());
}

#[tokio::test]
async fn a_service_forward_waits_without_a_pod_and_recovers_when_one_is_ready() {
    let cluster = FakeCluster::new(Some(web_service()), vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Service", "web", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut events = handle.events();

    cluster.push(vec![]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::TargetGone {
            pod: "web-1".into()
        }
    );
    // Still listening, but there is nothing behind it: the client is turned away.
    let mut refused = tokio::net::TcpStream::connect(handle.local_addr())
        .await
        .expect("accepts");
    let mut buf = [0u8; 1];
    let read = tokio::io::AsyncReadExt::read(&mut refused, &mut buf).await;
    assert!(
        matches!(read, Ok(0) | Err(_)),
        "closed without data: {read:?}"
    );
    assert!(connector.calls().is_empty(), "no pod was dialed");

    // A replacement that is not ready yet changes nothing...
    let mut starting = pod("web-9", 9);
    starting.ready = false;
    cluster.push(vec![starting.clone()]);
    // ...until it is.
    starting.ready = true;
    cluster.push(vec![starting]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::Listening {
            local_addr: handle.local_addr(),
            pod: "web-9".into()
        }
    );
    assert_eq!(echo_through(&handle, b"back").await, b"back");
}

#[tokio::test]
async fn a_service_forward_stays_on_its_pod_while_it_can_serve() {
    let cluster = FakeCluster::new(Some(web_service()), vec![pod("web-2", 2)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Service", "web", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut events = handle.events();

    // An older pod appears: no reason to move.
    cluster.push(vec![pod("web-1", 1), pod("web-2", 2)]);
    // Then web-2 turns unready: now it moves, in one TargetGone / Listening pair.
    let mut unready = pod("web-2", 2);
    unready.ready = false;
    cluster.push(vec![pod("web-1", 1), unready]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::TargetGone {
            pod: "web-2".into()
        }
    );
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::Listening {
            local_addr: handle.local_addr(),
            pod: "web-1".into()
        }
    );
}

#[tokio::test]
async fn a_pod_forward_reports_target_gone_then_stops_and_closes_its_port() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut events = handle.events();
    let addr = handle.local_addr();

    cluster.push(vec![]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::TargetGone {
            pod: "web-1".into()
        }
    );
    assert_eq!(next_event(&mut events).await, ForwardStatus::Stopped);
    wait_status(&handle, ForwardStatus::is_terminal).await;
    assert!(
        tokio::time::timeout(DEADLINE, async {
            while !handle.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok()
    );
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "the listener is closed"
    );
}

#[tokio::test]
async fn a_pod_forward_survives_losing_readiness_but_not_termination() {
    let cluster = FakeCluster::new(None, vec![pod("web-1", 1)]);
    let connector = FakeConnector::new([]);
    let handle = start(
        &cluster,
        &connector,
        &spec("Pod", "web-1", ForwardPort::Number(80)),
    )
    .await
    .expect("start");
    let mut events = handle.events();

    let mut sick = pod("web-1", 1);
    sick.ready = false;
    cluster.push(vec![sick.clone()]);
    assert_eq!(echo_through(&handle, b"still here").await, b"still here");

    sick.terminating = true;
    cluster.push(vec![sick]);
    assert_eq!(
        next_event(&mut events).await,
        ForwardStatus::TargetGone {
            pod: "web-1".into()
        }
    );
    assert_eq!(next_event(&mut events).await, ForwardStatus::Stopped);
}
