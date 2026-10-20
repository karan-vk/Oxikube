//! The manager over the fakes: starts, churn (the pod deleted then replaced, re-resolved at once
//! on the same local port), backoff on repeated failures, permanent errors, the read-only policy,
//! notifications with their dedupe keys, stop / restart / remove and the change subscription.

use std::time::Duration;

use oxikube_domain::notification::ForwardPhase;
use oxikube_ports::{PodRef, PortForwardError};
use oxikube_testkit::{PortForwardCall, SourceCall};
use serde_json::json;

use super::{NS, Rig, echo, pod, pod_spec, service, service_spec, slice};
use crate::portforward::model::{ForwardConfig, StartError};
use crate::portforward::state::ForwardState;

fn is_active(info: &crate::portforward::ForwardInfo) -> bool {
    matches!(info.state, ForwardState::Active { .. })
}

fn active_pod(info: &crate::portforward::ForwardInfo) -> Option<&str> {
    match &info.state {
        ForwardState::Active { pod, .. } => Some(pod),
        _ => None,
    }
}

fn web_service(rig: &Rig) {
    rig.backend.push(
        "Service",
        vec![service(
            "web",
            json!([{"name": "http", "port": 80, "targetPort": "http"}]),
        )],
    );
}

fn web_slices(rig: &Rig, endpoints: &[(&str, bool)]) {
    rig.backend.push(
        "EndpointSlice",
        vec![slice(
            "web-x",
            "web",
            json!([{"name": "http", "port": 8080}]),
            endpoints,
        )],
    );
}

fn notified(rig: &Rig) -> Vec<(ForwardPhase, ForwardPhase, String)> {
    rig.notes
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            SourceCall::PortForward(c) => Some((c.from, c.to, c.dedupe_key())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_pod_forward_goes_active_and_carries_bytes() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let id = rig.manager.start(pod_spec("api-0", 5432)).expect("start");
    let info = rig.until(id, is_active).await;
    let addr = info.local_addr.expect("bound");
    assert!(addr.ip().is_loopback(), "loopback by default");
    assert_eq!(echo(addr, b"select 1").await, b"select 1");
    let info = rig.until(id, |i| i.stats.bytes_in == 8).await;
    assert_eq!(info.stats.total_connections, 1);
    assert_eq!(info.pod.as_deref(), Some("api-0"));
    assert_eq!(info.label(), format!("pod/api-0 {}:5432", addr.port()));
    assert!(matches!(
        rig.port.recorded_calls().as_slice(),
        [PortForwardCall::Start { target, remote_port: 5432, local_port: None, .. }]
            if *target == PodRef::new(NS, "api-0")
    ));
}

#[tokio::test]
async fn a_service_forward_follows_churn_on_the_same_local_port_within_3s() {
    let rig = Rig::new();
    web_service(&rig);
    web_slices(&rig, &[("web-a", true), ("web-b", true)]);
    let id = rig.manager.start(service_spec("web", 80)).expect("start");
    let first = rig.until(id, is_active).await;
    assert_eq!(active_pod(&first), Some("web-a"));
    let addr = first.local_addr.unwrap();

    // Another pod joins: the forward stays on its pod.
    web_slices(&rig, &[("web-a", true), ("web-b", true), ("web-c", true)]);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(active_pod(&rig.manager.get(id).unwrap()), Some("web-a"));

    // The pod is deleted, then a replacement becomes ready.
    web_slices(&rig, &[("web-b", false)]);
    rig.until(id, |i| matches!(i.state, ForwardState::Reconnecting { .. }))
        .await;
    web_slices(&rig, &[("web-b", false), ("web-d", true)]);
    let took = rig
        .advance_until(
            id,
            Duration::from_millis(100),
            Duration::from_secs(3),
            |i| active_pod(i) == Some("web-d"),
        )
        .await;
    assert!(
        took < Duration::from_secs(3),
        "reconnected in {took:?} of fake time"
    );
    let now = rig.manager.get(id).unwrap();
    assert_eq!(
        now.local_addr,
        Some(addr),
        "the local port survives the reconnect"
    );
    assert_eq!(echo(addr, b"hi").await, b"hi");

    let calls = rig.port.recorded_calls();
    let PortForwardCall::Start {
        local_port,
        remote_port,
        ..
    } = calls.last().unwrap()
    else {
        panic!("a start: {calls:?}");
    };
    assert_eq!(
        *local_port,
        Some(addr.port()),
        "asked for the same port again"
    );
    assert_eq!(*remote_port, 8080, "the Service port's targetPort");

    let key = format!("portforward:{}:{id}:dropped", super::cluster());
    let notes = notified(&rig);
    assert!(
        notes.contains(&(
            ForwardPhase::Active,
            ForwardPhase::Reconnecting,
            key.clone()
        )),
        "the drop is notified: {notes:?}"
    );
    assert!(
        notes.contains(&(ForwardPhase::Reconnecting, ForwardPhase::Active, key)),
        "and the recovery: {notes:?}"
    );
}

#[tokio::test]
async fn pod_gone_from_the_forward_reconnects_at_once() {
    let rig = Rig::new();
    web_service(&rig);
    web_slices(&rig, &[("web-a", true), ("web-b", true)]);
    let id = rig.manager.start(service_spec("web", 80)).expect("start");
    let addr = rig.until(id, is_active).await.local_addr;
    // The adapter reports the pod gone before the watch does.
    web_slices(&rig, &[("web-b", true)]);
    rig.port
        .inject_error(&PortForwardError::PodGone(format!("{NS}/web-a")));
    let took = rig
        .advance_until(
            id,
            Duration::from_millis(100),
            Duration::from_secs(3),
            |i| active_pod(i) == Some("web-b"),
        )
        .await;
    assert!(took < Duration::from_secs(3));
    assert_eq!(rig.manager.get(id).unwrap().local_addr, addr);
}

#[tokio::test]
async fn repeated_failures_back_off_and_then_connect() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    for _ in 0..3 {
        rig.port
            .fail_next_start(PortForwardError::PodNotReady(format!("{NS}/api-0")));
    }
    let id = rig.manager.start(pod_spec("api-0", 80)).expect("start");
    let waiting = rig
        .until(id, |i| {
            matches!(i.state, ForwardState::Reconnecting { attempt: 2, .. })
        })
        .await;
    let ForwardState::Reconnecting { delay, .. } = waiting.state else {
        unreachable!()
    };
    assert!(delay >= Duration::from_millis(400), "backs off: {delay:?}");
    // Nothing happens until the clock moves.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(rig.port.recorded_calls().len(), 2);
    rig.advance_until(
        id,
        Duration::from_millis(250),
        Duration::from_secs(10),
        is_active,
    )
    .await;
    assert_eq!(rig.port.recorded_calls().len(), 4);
}

#[tokio::test]
async fn a_permanent_error_fails_with_its_suggestion() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    rig.port
        .fail_next_start(PortForwardError::Forbidden("shop/api-0".into()));
    let err = rig
        .manager
        .start_and_wait(pod_spec("api-0", 80))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StartError::Forward(PortForwardError::Forbidden(_))
    ));
    assert!(err.suggestion().unwrap().contains("pods/portforward"));
    let listed = rig.manager.list();
    assert_eq!(listed.len(), 1, "a failed forward stays listed");
    let ForwardState::Failed(failure) = &listed[0].state else {
        panic!("failed: {listed:?}");
    };
    assert!(failure.suggestion().is_some());
    assert!(
        notified(&rig)
            .iter()
            .all(|(from, to, _)| (*from, *to) == (ForwardPhase::Starting, ForwardPhase::Failed)),
        "a failed start is not a drop"
    );
}

#[tokio::test]
async fn a_busy_local_port_fails_the_start() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let err = rig
        .manager
        .start_and_wait(pod_spec("api-0", 80).with_local_port(port))
        .await
        .unwrap_err();
    assert!(err.suggestion().unwrap().contains("another port"), "{err}");
}

#[tokio::test]
async fn no_ready_pod_waits_then_gives_up() {
    let rig = Rig::new();
    rig.manager.set_config(ForwardConfig {
        backoff: crate::portforward::Backoff {
            max_attempts: 3,
            ..Default::default()
        },
        ..ForwardConfig::default()
    });
    web_service(&rig);
    web_slices(&rig, &[("web-a", false)]);
    let id = rig.manager.start(service_spec("web", 80)).expect("start");
    let waiting = rig
        .until(id, |i| matches!(i.state, ForwardState::Reconnecting { .. }))
        .await;
    assert!(waiting.state.reason().unwrap().contains("no ready pod"));
    rig.advance_until(
        id,
        Duration::from_millis(500),
        Duration::from_secs(10),
        |i| matches!(i.state, ForwardState::Failed(_)),
    )
    .await;
    assert!(
        rig.port.recorded_calls().is_empty(),
        "never started without a pod"
    );
}

#[tokio::test]
async fn read_only_clusters_refuse_unless_allowed() {
    let rig = Rig::new();
    *rig.backend.read_only.lock() = true;
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let err = rig.manager.start(pod_spec("api-0", 80)).unwrap_err();
    assert!(matches!(err, StartError::ReadOnly(_)));
    assert!(err.suggestion().unwrap().contains("allowed_in_readonly"));
    assert!(
        rig.manager.list().is_empty(),
        "a refused start is not listed"
    );
    assert_eq!(rig.backend.watchers("Pod"), 0);

    rig.manager.set_config(ForwardConfig {
        allowed_in_readonly: true,
        ..ForwardConfig::default()
    });
    let id = rig.manager.start(pod_spec("api-0", 80)).expect("allowed");
    rig.until(id, is_active).await;
}

#[tokio::test]
async fn stop_restart_and_remove() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let mut changes = rig.manager.subscribe();
    let id = rig.manager.start(pod_spec("api-0", 80)).expect("start");
    let addr = rig.until(id, is_active).await.local_addr.unwrap();
    assert!(changes.changed().await, "changes are signalled");

    assert!(rig.manager.stop(id));
    assert_eq!(rig.manager.get(id).unwrap().state, ForwardState::Stopped);
    for _ in 0..100 {
        if rig.backend.watchers("Pod") == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(rig.backend.watchers("Pod"), 0, "the watch is released");
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "the listener closed"
    );

    rig.manager.restart(id).expect("restart");
    let again = rig.until(id, is_active).await;
    assert_eq!(
        again.local_addr,
        Some(addr),
        "a restart keeps the local port"
    );

    assert!(rig.manager.remove(id));
    assert!(rig.manager.get(id).is_none());
    assert!(!rig.manager.remove(id));
    assert!(matches!(
        rig.manager.restart(id),
        Err(StartError::Unknown(_))
    ));
    assert!(
        !notified(&rig)
            .iter()
            .any(|(_, to, _)| *to == ForwardPhase::Reconnecting),
        "stops are not drops"
    );
}

#[tokio::test]
async fn forwards_span_clusters_and_dropping_the_manager_stops_them() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let mut other = pod_spec("api-0", 80);
    other.target.cluster = oxikube_domain::ids::ClusterId::new(
        "kubeconfig",
        &oxikube_domain::ids::ContextName::from("prod"),
    );
    let a = rig.manager.start(pod_spec("api-0", 80)).unwrap();
    let b = rig.manager.start(other).unwrap();
    let addr = rig.until(a, is_active).await.local_addr.unwrap();
    rig.until(b, is_active).await;
    assert_eq!(rig.manager.list().len(), 2);
    drop(rig.manager);
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the forwards outlived the manager");
}

#[tokio::test]
async fn unsupported_targets_are_refused() {
    let rig = Rig::new();
    let mut spec = pod_spec("api", 80);
    spec.target.gvk = oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment");
    assert!(matches!(
        rig.manager.start(spec),
        Err(StartError::Unsupported(_))
    ));
}

#[tokio::test]
async fn start_and_wait_does_not_hang_on_a_feed_that_never_lists() {
    let rig = Rig::new();
    let wait = rig.manager.start_and_wait(pod_spec("api-0", 80));
    let advance = async {
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            rig.clock.advance(Duration::from_secs(1));
        }
    };
    let (info, ()) = futures::join!(wait, advance);
    assert_eq!(info.expect("still starting").state, ForwardState::Starting);
}
