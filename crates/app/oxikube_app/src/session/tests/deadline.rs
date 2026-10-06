//! `connect_with_deadline`: a limit on the whole attempt, on the injected clock.

use std::time::Duration;

use futures::FutureExt;
use oxikube_domain::OxiError;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};

use super::{Harness, id};

const LIMIT: Duration = Duration::from_secs(5);

#[test]
fn an_attempt_inside_the_limit_connects_as_usual() {
    let h = Harness::new();
    let state = h
        .manager
        .connect_with_deadline(&id("a"), LIMIT)
        .now_or_never()
        .expect("answers at once")
        .expect("connect");
    assert_eq!(state, ClusterSessionState::Ready);
}

#[test]
fn an_attempt_past_the_limit_is_cancelled_and_ends_in_error() {
    let h = Harness::new();
    h.connector.hold();
    let a = id("a");
    let mut connect = h.manager.connect_with_deadline(&a, LIMIT).boxed();
    assert!((&mut connect).now_or_never().is_none());
    assert_eq!(h.phase("a"), SessionPhase::Connecting);

    h.clock.advance(LIMIT);
    let state = connect.now_or_never().expect("done").expect("connect");

    assert_eq!(state.phase(), SessionPhase::Error);
    assert!(
        state
            .reason()
            .unwrap()
            .contains("did not answer within 5 s")
    );
    assert_eq!(h.connector.cancelled(), 1);
    assert_eq!(h.connector.live_connections(&a), 0);
}

#[test]
fn the_limit_covers_the_retries_and_their_backoff() {
    let h = Harness::new();
    h.connector
        .script()
        .connect
        .push_err(OxiError::network("connection refused"))
        .push_err(OxiError::network("connection refused"));
    let a = id("a");
    let mut connect = h.manager.connect_with_deadline(&a, LIMIT).boxed();
    assert!((&mut connect).now_or_never().is_none(), "backing off");

    // Both failed attempts and their backoff (0.5 s, then 1 s) would still fit in the retry
    // policy, but the limit is on the whole attempt: it ends it with a timeout, not a connect.
    h.clock.advance(LIMIT);
    let state = connect.now_or_never().expect("done").expect("connect");
    assert_eq!(state.phase(), SessionPhase::Error);
    assert!(state.reason().unwrap().contains("did not answer"));
}
