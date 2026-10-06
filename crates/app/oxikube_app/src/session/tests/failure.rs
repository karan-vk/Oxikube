//! Fatal errors, transient errors with backoff, and recovery from `Error`.

use std::time::Duration;

use futures::FutureExt;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{Capabilities, OxiError};
use oxikube_testkit::ClockCall;

use super::{Harness, id};
use crate::session::{RetryPolicy, SessionManagerConfig};

#[test]
fn a_fatal_error_ends_in_error_without_retrying() {
    let mut h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::validation("context `a` has no cluster entry"));
    let state = h.connect("a");
    assert_eq!(state.phase(), SessionPhase::Error);
    assert!(state.reason().unwrap().contains("no cluster entry"));
    assert_eq!(h.connector.recorded_calls().len(), 1);
    assert_eq!(
        h.phases("a"),
        [SessionPhase::Connecting, SessionPhase::Error]
    );
}

#[test]
fn transient_errors_are_retried_with_backoff() {
    let mut h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::network("connection refused"))
        .push_err(OxiError::timeout("dial timeout"));

    let a = id("a");
    let mut connect = h.manager.connect(&a).boxed();
    assert!(
        (&mut connect).now_or_never().is_none(),
        "waits for the first backoff"
    );
    assert_eq!(h.phase("a"), SessionPhase::Connecting);
    h.clock.advance(Duration::from_millis(500));
    assert!(
        (&mut connect).now_or_never().is_none(),
        "waits for the second backoff"
    );
    h.clock.advance(Duration::from_secs(1));
    let state = connect.now_or_never().expect("done").unwrap();
    assert_eq!(state, ClusterSessionState::Ready);
    assert_eq!(
        h.clock.recorded_calls(),
        [
            ClockCall::Sleep(Duration::from_millis(500)),
            ClockCall::Sleep(Duration::from_secs(1))
        ]
    );
    assert_eq!(h.connector.recorded_calls().len(), 3);
    // Retries happen inside Connecting: no intermediate states are announced.
    assert_eq!(
        h.phases("a"),
        [SessionPhase::Connecting, SessionPhase::Ready]
    );
}

#[test]
fn exhausted_retries_end_in_error() {
    let h = Harness::with_config(SessionManagerConfig {
        retry: RetryPolicy::no_retry(),
        ..SessionManagerConfig::default()
    });
    h.connector
        .script()
        .connect
        .push_err(OxiError::network("connection refused"));
    let state = h.connect("a");
    assert_eq!(state.phase(), SessionPhase::Error);
    assert!(state.reason().unwrap().contains("connection refused"));
}

#[test]
fn error_can_be_retried_with_connect_or_reconnect() {
    let h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::internal("boom"))
        .push_err(OxiError::internal("boom again"));
    assert_eq!(h.connect("a").phase(), SessionPhase::Error);
    assert_eq!(h.connect("a").phase(), SessionPhase::Error);
    assert_eq!(h.reconnect("a"), ClusterSessionState::Ready);
}

#[test]
fn a_failed_capability_probe_does_not_block_ready() {
    let h = Harness::new();
    h.connector
        .ports_for(&id("a"))
        .access
        .script()
        .capabilities
        .push_err(OxiError::network("rules review timed out").with_retryable(false));
    assert_eq!(h.connect("a"), ClusterSessionState::Ready);
    // Unknown is never "denied".
    assert_eq!(
        h.manager.get(&id("a")).unwrap().capabilities(),
        Capabilities::all()
    );
}

#[test]
fn reconnect_replaces_a_live_connection() {
    let mut h = Harness::new();
    h.connect("a");
    h.drain();
    assert_eq!(h.reconnect("a"), ClusterSessionState::Ready);
    assert_eq!(
        h.phases("a"),
        [
            SessionPhase::Disconnected,
            SessionPhase::Connecting,
            SessionPhase::Ready
        ]
    );
    assert_eq!(h.connector.recorded_calls().len(), 2);
    assert_eq!(h.connector.live_connections(&id("a")), 1);
}
