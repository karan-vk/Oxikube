//! Cancel on drop: dropping a session aborts its task and drops the port's stream.

use super::{Harness, burst};
use crate::logs::{EndReason, LogState, LogTarget};
use oxikube_ports::LogOptions;

#[test]
fn dropping_the_session_drops_the_ports_stream() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(5).keep_open());
    assert_eq!(h.port.live_streams(), 1);
    let reader = session.reader();
    assert_eq!(reader.state(), LogState::Streaming);

    drop(session);
    h.settle();
    assert_eq!(h.port.live_streams(), 0, "the stream was not aborted");
    // What was read stays readable for whoever kept a reader, and says it was cancelled.
    assert_eq!(reader.len(), 5);
    assert_eq!(reader.state(), LogState::Ended(EndReason::Cancelled));
}

#[test]
fn a_session_dropped_before_its_task_ran_never_opens_the_stream() {
    let h = Harness::new();
    h.port.script().stream_logs.push_ok(burst(1));
    let session = h.service.open(
        h.port.clone(),
        LogTarget::pod("default", "web-0"),
        LogOptions::default(),
    );
    drop(session);
    let mut h = h;
    h.settle();
    assert!(
        h.port.recorded_calls().is_empty(),
        "the aborted driver still opened the stream: {:?}",
        h.port.recorded_calls()
    );
    assert_eq!(h.port.live_streams(), 0);
}

#[test]
fn each_session_cancels_on_its_own() {
    let mut h = Harness::new();
    let a = h.follow_flushed(burst(1).keep_open());
    let b = h.follow_flushed(burst(1).keep_open());
    assert_eq!(h.port.live_streams(), 2);
    drop(a);
    h.settle();
    assert_eq!(h.port.live_streams(), 1);
    assert_eq!(b.state(), LogState::Streaming);
}

#[test]
fn a_session_that_ended_on_its_own_keeps_its_end_reason_when_dropped() {
    let mut h = Harness::new();
    let session = h.open(
        burst(2),
        LogTarget::pod("default", "web-0"),
        LogOptions::default(),
    );
    let reader = session.reader();
    drop(session);
    assert_eq!(reader.state(), LogState::Ended(EndReason::Completed));
}
