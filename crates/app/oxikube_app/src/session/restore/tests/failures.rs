//! Failures are isolated per cluster, and none of them blocks the rest.

use std::pin::pin;

use oxikube_domain::OxiError;

use super::*;
use crate::session::restore::{RestoreConnect, RestorePlan};

fn prepared(h: &Harness, open: &[&str], active: Option<&str>) -> RestorePlan {
    h.save(open, active);
    block_on(h.restorer.prepare()).expect("prepare")
}

#[test]
fn a_failing_cluster_does_not_block_the_others() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b", "c"], Some("a"));
    h.connector
        .connect_script_for(&id("a"))
        .push_err(OxiError::unsupported(
            "the API server refused the handshake",
        ));

    let report = block_on(h.restorer.connect(&plan, RestoreConnect::All));

    assert_eq!(h.phase("a"), SessionPhase::Error);
    assert_eq!(h.phase("b"), SessionPhase::Ready);
    assert_eq!(h.phase("c"), SessionPhase::Ready);
    let failed: Vec<_> = report.failures().map(|o| o.cluster.clone()).collect();
    assert_eq!(failed, [id("a")]);
    match h.state_of("a") {
        ClusterSessionState::Error { reason } => assert!(reason.contains("handshake"), "{reason}"),
        other => panic!("expected an error state, got {other:?}"),
    }
}

#[test]
fn a_cluster_that_needs_credentials_is_reported_not_retried() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b"], Some("a"));
    h.connector
        .connect_script_for(&id("a"))
        .push_err(OxiError::auth("token expired: run `aws sso login`", false));

    let report = block_on(h.restorer.connect(&plan, RestoreConnect::All));

    assert_eq!(h.phase("a"), SessionPhase::AuthRequired);
    assert_eq!(h.phase("b"), SessionPhase::Ready);
    assert!(report.outcome(&id("a")).is_some_and(|o| o.failed()));
    assert_eq!(h.connects(), ["a", "b"], "one attempt each");
}

#[test]
fn the_deadline_cancels_the_attempt_and_shows_the_timeout_as_the_error() {
    let h = Harness::new();
    let plan = prepared(&h, &["a", "b"], Some("a"));
    h.connector.hold();

    let mut fut = pin!(h.restorer.connect(&plan, RestoreConnect::Active));
    assert!(!poll_once(&mut fut));
    assert_eq!(h.phase("a"), SessionPhase::Connecting);

    h.clock.advance(TIMEOUT);
    let report = block_on(fut);

    assert_eq!(h.phase("a"), SessionPhase::Error);
    match h.state_of("a") {
        ClusterSessionState::Error { reason } => {
            assert!(reason.contains("did not answer within 30 s"), "{reason}")
        }
        other => panic!("expected an error state, got {other:?}"),
    }
    assert_eq!(h.connector.cancelled(), 1, "the hung connect was dropped");
    assert!(report.outcome(&id("a")).is_some_and(|o| o.failed()));
}

#[test]
fn a_timeout_frees_its_slot_for_the_next_cluster() {
    let h = Harness::with_config(RestoreConfig {
        concurrency: 1,
        connect_timeout: TIMEOUT,
    });
    let plan = prepared(&h, &["a", "b"], Some("a"));
    // The dead VPN: connects hang until released.
    h.connector.hold();

    let mut fut = pin!(h.restorer.connect(&plan, RestoreConnect::All));
    assert!(!poll_once(&mut fut));
    assert_eq!(h.connects(), ["a"], "b waits for the single slot");

    h.clock.advance(TIMEOUT);
    assert!(!poll_once(&mut fut), "b is now in flight, still held");
    assert_eq!(h.phase("a"), SessionPhase::Error, "a gave up on its own");
    assert_eq!(h.connects(), ["a", "b"]);

    h.connector.release();
    block_on(fut);
    assert_eq!(h.phase("a"), SessionPhase::Error);
    assert_eq!(h.phase("b"), SessionPhase::Ready);
}

#[test]
fn an_unreadable_catalog_reopens_and_drops_nothing() {
    let h = Harness::new();
    h.save(&["a", "b"], Some("a"));
    h.source
        .script()
        .contexts
        .push_err(OxiError::internal("the kubeconfig directory is unreadable"));

    let result = block_on(h.restorer.prepare());

    assert!(result.is_err());
    assert!(h.open_names().is_empty());
    let saved = h.saved().expect("the saved session is untouched");
    assert_eq!(saved.open, [id("a"), id("b")]);
}
