//! Cancelling an in-flight connect: `disconnect`, `reconnect`, `close`, or dropping the
//! future.

use std::time::Duration;

use futures::FutureExt;
use oxikube_domain::OxiError;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};

use super::{Harness, id};

#[test]
fn disconnect_while_connecting_aborts_the_in_flight_connect() {
    let mut h = Harness::new();
    let a = id("a");
    h.connector.hold();
    let mut connect = h.manager.connect(&a).boxed();
    assert!((&mut connect).now_or_never().is_none());
    assert_eq!(h.phase("a"), SessionPhase::Connecting);
    assert_eq!(h.connector.held(), 1);

    h.manager.disconnect(&a).unwrap();
    assert_eq!(h.phase("a"), SessionPhase::Disconnected);
    // The caller's future finishes with the state it was cancelled into, and the
    // connector's future was dropped without completing.
    let state = connect.now_or_never().expect("aborted").unwrap();
    assert_eq!(state, ClusterSessionState::Disconnected);
    assert_eq!(h.connector.cancelled(), 1);
    assert_eq!(h.connector.live_connections(&a), 0);
    assert_eq!(
        h.phases("a"),
        [SessionPhase::Connecting, SessionPhase::Disconnected]
    );

    // Releasing later changes nothing.
    h.connector.release();
    assert_eq!(h.phase("a"), SessionPhase::Disconnected);
}

#[test]
fn disconnect_during_backoff_cancels_the_retry() {
    let h = Harness::new();
    let a = id("a");
    h.connector
        .script()
        .connect
        .push_err(OxiError::network("refused"));
    let mut connect = h.manager.connect(&a).boxed();
    assert!((&mut connect).now_or_never().is_none());
    assert_eq!(h.clock.pending_sleepers(), 1);
    h.manager.disconnect(&a).unwrap();
    assert_eq!(
        connect.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Disconnected
    );
    assert_eq!(h.clock.pending_sleepers(), 0);
    h.clock.advance(Duration::from_secs(60));
    assert_eq!(h.connector.recorded_calls().len(), 1);
}

#[test]
fn dropping_the_connect_future_returns_to_disconnected() {
    let mut h = Harness::new();
    let a = id("a");
    h.connector.hold();
    let mut connect = h.manager.connect(&a).boxed();
    assert!((&mut connect).now_or_never().is_none());
    drop(connect);
    assert_eq!(h.phase("a"), SessionPhase::Disconnected);
    assert_eq!(h.connector.cancelled(), 1);
    assert_eq!(
        h.phases("a"),
        [SessionPhase::Connecting, SessionPhase::Disconnected]
    );
}

#[test]
fn reconnect_while_connecting_restarts_the_attempt() {
    let h = Harness::new();
    let a = id("a");
    h.connector.hold();
    let mut first = h.manager.connect(&a).boxed();
    assert!((&mut first).now_or_never().is_none());

    let mut second = h.manager.reconnect(&a).boxed();
    assert!((&mut second).now_or_never().is_none());
    // The superseded caller is woken by the abort and sees the restart, not a result of
    // its own; its connector call is dropped.
    assert_eq!(
        first.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Connecting
    );
    assert_eq!(h.connector.cancelled(), 1);

    h.connector.release();
    assert_eq!(
        second.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Ready
    );
    assert_eq!(h.connector.live_connections(&a), 1);
}

#[test]
fn a_second_connect_while_connecting_does_not_start_another_attempt() {
    let h = Harness::new();
    let a = id("a");
    h.connector.hold();
    let mut first = h.manager.connect(&a).boxed();
    assert!((&mut first).now_or_never().is_none());
    assert_eq!(h.connect("a"), ClusterSessionState::Connecting);
    assert_eq!(h.connector.recorded_calls().len(), 1);
    h.connector.release();
    assert_eq!(
        first.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Ready
    );
}

#[test]
fn closing_while_connecting_drops_the_late_result() {
    let h = Harness::new();
    let a = id("a");
    h.connector.hold();
    let mut connect = h.manager.connect(&a).boxed();
    assert!((&mut connect).now_or_never().is_none());
    assert!(h.manager.close(&a));
    assert_eq!(
        connect.now_or_never().unwrap().unwrap(),
        ClusterSessionState::Disconnected
    );
    assert!(h.manager.get(&a).is_none());
    assert_eq!(h.connector.live_connections(&a), 0);
}
