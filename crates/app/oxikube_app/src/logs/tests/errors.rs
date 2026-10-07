//! Error paths: a stream that ends, one that breaks, and a pod that is not there.

use std::time::Duration;

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::LogOptions;
use oxikube_testkit::Timeline;

use super::{Harness, burst, line};
use crate::logs::{EndReason, LogConfig, LogFailure, LogState, LogTarget, ReconnectPolicy};

fn failed(state: LogState) -> LogFailure {
    match state {
        LogState::Failed(failure) => failure,
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn a_session_starts_connecting_and_streams_once_the_port_answers() {
    let mut h = Harness::new();
    h.port.script().stream_logs.push_ok(burst(1).keep_open());
    let session = h.service.open(
        h.port.clone(),
        LogTarget::pod("default", "web-0"),
        LogOptions::follow(),
    );
    assert_eq!(
        session.state(),
        LogState::Connecting,
        "open never waits for the port"
    );
    h.settle();
    assert_eq!(session.state(), LogState::Streaming);
}

#[test]
fn a_pod_that_is_not_found_is_a_failed_session() {
    let mut h = Harness::new();
    h.port
        .script()
        .stream_logs
        .push_err(OxiError::not_found("pod default/gone not found"));
    let session = h.service.open(
        h.port.clone(),
        LogTarget::pod("default", "gone"),
        LogOptions::follow(),
    );
    h.settle();
    let failure = failed(session.state());
    assert_eq!(failure.kind, ErrorKind::NotFound);
    assert!(!failure.retryable);
    assert!(failure.message.contains("gone"));
    assert!(session.is_empty());
}

#[test]
fn forbidden_and_unstarted_containers_are_failures_with_their_kinds() {
    let mut h = Harness::new();
    for (error, kind) in [
        (
            OxiError::forbidden("pods/log is forbidden"),
            ErrorKind::Forbidden,
        ),
        (
            OxiError::validation("container \"app\" is waiting to start"),
            ErrorKind::Validation,
        ),
    ] {
        h.port.script().stream_logs.push_err(error);
        let session = h.service.open(
            h.port.clone(),
            LogTarget::pod("default", "web-0"),
            LogOptions::default(),
        );
        h.settle();
        assert_eq!(failed(session.state()).kind, kind);
    }
}

#[test]
fn an_unscripted_port_is_a_failed_session_not_a_panic() {
    let mut h = Harness::new();
    let session = h.service.open(
        h.port.clone(),
        LogTarget::pod("default", "web-0"),
        LogOptions::default(),
    );
    h.settle();
    assert_eq!(failed(session.state()).kind, ErrorKind::Internal);
}

#[test]
fn a_read_that_reaches_the_end_is_completed() {
    let mut h = Harness::new();
    let session = h.open(
        burst(3),
        LogTarget::pod("default", "web-0"),
        LogOptions::default().tail_lines(3),
    );
    assert_eq!(session.state(), LogState::Ended(EndReason::Completed));
    assert_eq!(session.len(), 3, "the lines read stay after the end");
}

#[test]
fn a_followed_stream_that_ends_is_closed_not_completed() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(2));
    assert_eq!(session.state(), LogState::Ended(EndReason::StreamClosed));
    assert_eq!(session.len(), 2);
}

#[test]
fn a_stream_that_breaks_keeps_its_lines_and_reports_a_retryable_failure() {
    // Without reconnects (E08-S07 reconnects by default: `churn::tests`).
    let mut h = Harness::with_config(LogConfig {
        reconnect: ReconnectPolicy::Never,
        ..LogConfig::default()
    });
    let session = h.follow(
        Timeline::new()
            .ok_at(Duration::ZERO, line(0))
            .ok_at(Duration::ZERO, line(1))
            .err_at(
                Duration::from_millis(5),
                OxiError::network("connection reset"),
            ),
    );
    h.advance(Duration::from_millis(10));
    assert_eq!(session.len(), 2, "lines before the error are committed");
    let failure = failed(session.state());
    assert_eq!(failure.kind, ErrorKind::Network);
    assert!(failure.retryable);
}

#[test]
fn a_terminal_state_is_final() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(1));
    assert_eq!(session.state(), LogState::Ended(EndReason::StreamClosed));
    h.advance(Duration::from_secs(60));
    assert_eq!(session.state(), LogState::Ended(EndReason::StreamClosed));
}

#[test]
fn a_failure_message_is_redacted() {
    let mut h = Harness::new();
    h.port.script().stream_logs.push_err(OxiError::network(
        "request failed: Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
    ));
    let session = h.service.open(
        h.port.clone(),
        LogTarget::pod("default", "web-0"),
        LogOptions::default(),
    );
    h.settle();
    let failure = failed(session.state());
    assert!(
        !failure.message.contains("abcdefghijklmnop"),
        "{}",
        failure.message
    );
}
