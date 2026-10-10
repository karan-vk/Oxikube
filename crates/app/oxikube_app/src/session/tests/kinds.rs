//! Following the cluster's kinds while connected (E03-F544): the manager starts the adapter's
//! subscription on connect, turns its events into session updates, and stops it with the
//! connection. These tests need a runtime for the forwarder task (current-thread, so a
//! `yield_now` lets it run).

use oxikube_domain::ids::Gvk;
use oxikube_ports::{CrdWatchStatus, DiscoveryEvent, KindsChange};
use oxikube_testkit::{DiscoveryCall, SchemaCall};

use super::{Harness, SessionChange, id};

/// Lets the forwarder task run.
async fn settle() {
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
}

fn added(kind: &str) -> KindsChange {
    KindsChange {
        added: vec![Gvk::new("example.io", "v1", kind)],
        ..KindsChange::default()
    }
}

fn forbidden() -> CrdWatchStatus {
    CrdWatchStatus::Forbidden {
        reason: "customresourcedefinitions is forbidden".into(),
    }
}

#[tokio::test]
async fn connecting_starts_the_watch_through_the_port() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    let ports = h.connector.ports_for(&a);
    assert_eq!(
        ports.discovery.recorded_calls(),
        [DiscoveryCall::Discover, DiscoveryCall::Subscribe]
    );
    assert_eq!(ports.discovery.live_subscriptions(), 1);
    assert_eq!(
        h.manager.get(&a).unwrap().crd_watch(),
        &CrdWatchStatus::Watching
    );
}

#[tokio::test]
async fn a_crd_added_while_connected_becomes_a_session_update() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.drain();

    let discovery = h.connector.ports_for(&a).discovery;
    discovery.emit(DiscoveryEvent::KindsChanged(added("Widget")));
    settle().await;

    let changes: Vec<_> = h
        .drain()
        .into_iter()
        .filter(|u| u.cluster == a)
        .map(|u| u.change)
        .collect();
    assert_eq!(changes, [SessionChange::KindsChanged(added("Widget"))]);
}

#[tokio::test]
async fn a_kinds_change_invalidates_the_connections_schemas_but_a_watch_status_does_not() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    let ports = h.connector.ports_for(&a);

    ports.discovery.emit(DiscoveryEvent::CrdWatch(forbidden()));
    settle().await;
    assert!(ports.schemas.recorded_calls().is_empty());

    ports
        .discovery
        .emit(DiscoveryEvent::KindsChanged(added("Widget")));
    settle().await;
    assert_eq!(ports.schemas.recorded_calls(), [SchemaCall::Invalidate(a)]);
}

#[tokio::test]
async fn a_refused_watch_is_visible_on_the_session_and_announced_once() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.drain();

    let discovery = h.connector.ports_for(&a).discovery;
    discovery.emit(DiscoveryEvent::CrdWatch(forbidden()));
    discovery.emit(DiscoveryEvent::CrdWatch(forbidden()));
    settle().await;

    assert_eq!(h.manager.get(&a).unwrap().crd_watch(), &forbidden());
    let changes: Vec<_> = h.drain().into_iter().map(|u| u.change).collect();
    assert_eq!(changes, [SessionChange::CrdWatchChanged(forbidden())]);

    discovery.emit(DiscoveryEvent::CrdWatch(CrdWatchStatus::Watching));
    settle().await;
    assert_eq!(
        h.manager.get(&a).unwrap().crd_watch(),
        &CrdWatchStatus::Watching
    );
}

#[tokio::test]
async fn disconnecting_stops_the_watch_and_clears_the_status() {
    let mut h = Harness::new();
    let a = id("a");
    h.connect("a");
    let discovery = h.connector.ports_for(&a).discovery;
    discovery.emit(DiscoveryEvent::CrdWatch(forbidden()));
    settle().await;
    h.drain();

    h.manager.disconnect(&a).unwrap();
    settle().await;
    assert_eq!(
        discovery.live_subscriptions(),
        0,
        "the subscription (and the adapter's watch) is dropped"
    );
    assert_eq!(
        h.manager.get(&a).unwrap().crd_watch(),
        &CrdWatchStatus::Watching
    );
    assert!(
        h.drain()
            .iter()
            .any(|u| u.change == SessionChange::CrdWatchChanged(CrdWatchStatus::Watching))
    );

    // Nothing arrives for a session that is not connected.
    discovery.emit(DiscoveryEvent::KindsChanged(added("Late")));
    settle().await;
    assert!(h.drain().is_empty());
}

#[tokio::test]
async fn reconnecting_replaces_the_watch() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    h.reconnect("a");
    settle().await;
    let discovery = h.connector.ports_for(&a).discovery;
    assert_eq!(discovery.live_subscriptions(), 1);
    assert_eq!(
        discovery
            .recorded_calls()
            .iter()
            .filter(|c| **c == DiscoveryCall::Subscribe)
            .count(),
        2
    );
}

#[tokio::test]
async fn closing_the_session_stops_the_watch() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    let discovery = h.connector.ports_for(&a).discovery;
    h.manager.close(&a);
    settle().await;
    assert_eq!(discovery.live_subscriptions(), 0);
}

#[test]
fn without_a_runtime_the_session_still_connects_and_follows_nothing() {
    let h = Harness::new();
    let a = id("a");
    h.connect("a");
    assert!(h.manager.get(&a).unwrap().is_connected());
    assert_eq!(h.connector.ports_for(&a).discovery.live_subscriptions(), 0);
}
