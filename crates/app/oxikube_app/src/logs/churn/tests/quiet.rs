//! A followed stream that closes while its pod runs but brings no new line: a quiet pod behind a
//! proxy that closes idle streams, a container that completed next to a live one, a container
//! between restarts. None of them may wear out the retry count.

use std::time::Duration;

use oxikube_ports::LogOptions;
use oxikube_testkit::Timeline;
use serde_json::{Value, json};

use super::{Harness, line, lines, owned_pod, resource, texts};
use crate::logs::{EndReason, LogState};

/// `web-0` (owned by a ReplicaSet, phase `Running`) with the container statuses `app_state`
/// for `app` and a completed init container `init`.
fn pod_with(app_state: Value) -> Value {
    let mut pod = owned_pod("web-0", "u1", Some(("ReplicaSet", "web-5d8")), "Running");
    pod["spec"]["initContainers"] = json!([{"name": "init"}]);
    pod["status"]["initContainerStatuses"] = json!([{
        "name": "init", "restartCount": 0,
        "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}
    }]);
    pod["status"]["containerStatuses"] = json!([{
        "name": "app", "restartCount": 5, "state": app_state
    }]);
    pod
}

/// Line 0 at once, then the end of the stream.
fn one_line() -> Timeline<oxikube_domain::log::LogLine> {
    lines(0..1)
}

#[test]
fn a_quiet_stream_that_stays_open_does_not_count_as_a_failure() {
    let mut h = Harness::new();
    h.service.set_reconnect_retries(1);
    h.resources
        .insert(resource(pod_with(json!({"running": {}}))));
    h.script(one_line());
    // An idle-timeout proxy closes each reopened stream after a minute; it replays line 0 and
    // brings nothing new.
    for _ in 0..4 {
        h.script(Timeline::new().ok_at(Duration::from_secs(60), line(0)));
    }
    h.script(lines(0..3).keep_open());
    let session = h.follow(LogOptions::follow());
    for _ in 0..5 {
        h.run_for(Duration::from_secs(62));
        assert!(
            !matches!(session.state(), LogState::Failed(_)),
            "a healthy pod's session never fails: {:?}",
            session.state()
        );
    }
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), ["line 0", "line 1", "line 2"]);
    assert_eq!(h.opens().len(), 6);
}

#[test]
fn a_completed_init_container_in_a_running_pod_has_ended() {
    let mut h = Harness::new();
    h.resources
        .insert(resource(pod_with(json!({"running": {}}))));
    h.script(lines(0..2));
    let session = h.follow(LogOptions::follow().container("init"));
    h.run_for(Duration::from_secs(20));
    assert_eq!(
        session.state(),
        LogState::Ended(EndReason::ContainerFinished)
    );
    assert_eq!(h.opens().len(), 1, "a finished container is not reopened");
    assert_eq!(texts(&session), ["line 0", "line 1"]);
}

#[test]
fn a_crash_looping_container_is_waited_for_without_counting_failures() {
    let mut h = Harness::new();
    h.service.set_reconnect_retries(1);
    let waiting = json!({"waiting": {"reason": "CrashLoopBackOff"}});
    h.resources.insert(resource(pod_with(waiting)));
    h.script(one_line());
    // Each reopen reads the terminated instance's log again (line 0) and closes.
    for _ in 0..4 {
        h.script(one_line());
    }
    // The container starts again and writes.
    h.script(lines(0..3).keep_open());
    let session = h.follow(LogOptions::follow());
    h.run_for(Duration::from_millis(100));
    assert_eq!(
        session.state(),
        LogState::Connecting,
        "waiting for the next instance, not reconnecting"
    );
    h.run_for(Duration::from_secs(25));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), ["line 0", "line 1", "line 2"]);
    assert_eq!(h.opens().len(), 6);
}
