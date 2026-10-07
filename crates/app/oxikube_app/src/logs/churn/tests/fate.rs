//! A single-pod session says why its stream ended: the pod finished, was replaced (a controller
//! owns it) or deleted; and a pod that still runs is reconnected.

use std::time::Duration;

use oxikube_domain::ids::Gvk;
use oxikube_ports::LogOptions;
use oxikube_testkit::Timeline;

use super::{Harness, line, lines, owned_pod, resource, texts};
use crate::logs::{EndReason, LogState};

/// Two lines, the second at 200 ms, then the end of the stream.
fn short() -> Timeline<oxikube_domain::log::LogLine> {
    Timeline::new()
        .ok_at(Duration::ZERO, line(0))
        .ok_at(Duration::from_millis(200), line(1))
}

fn pod_kind() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

#[test]
fn a_pod_replaced_by_a_rollout_ends_replaced_with_its_controller_known() {
    let mut h = Harness::new();
    h.resources.insert(resource(owned_pod(
        "web-0",
        "u1",
        Some(("ReplicaSet", "web-5d8")),
        "Running",
    )));
    h.script(short());
    let session = h.follow(LogOptions::follow());
    let identity = session.pod_identity().expect("read as the stream opened");
    assert_eq!(
        identity.controller.as_ref().map(|c| &*c.name),
        Some("web-5d8")
    );
    // The rollout deletes the pod; its stream ends with its container.
    let mut going = owned_pod("web-0", "u1", Some(("ReplicaSet", "web-5d8")), "Running");
    going["metadata"]["deletionTimestamp"] = serde_json::json!("2026-10-07T12:00:00Z");
    h.resources.insert(resource(going));
    h.run_for(Duration::from_millis(300));
    let state = session.state();
    assert_eq!(state, LogState::Ended(EndReason::PodReplaced));
    assert!(matches!(state, LogState::Ended(reason) if reason.has_replacement()));
    assert_eq!(texts(&session), ["line 0", "line 1"], "its lines stay");
}

#[test]
fn a_pod_that_ran_to_its_end_is_finished() {
    let mut h = Harness::new();
    h.resources.insert(resource(owned_pod(
        "web-0",
        "u1",
        Some(("Job", "migrate")),
        "Running",
    )));
    h.script(short());
    let session = h.follow(LogOptions::follow());
    h.resources.insert(resource(owned_pod(
        "web-0",
        "u1",
        Some(("Job", "migrate")),
        "Succeeded",
    )));
    h.run_for(Duration::from_millis(300));
    assert_eq!(session.state(), LogState::Ended(EndReason::PodFinished));
}

#[test]
fn a_bare_pod_that_is_deleted_has_no_replacement() {
    let mut h = Harness::new();
    h.resources
        .insert(resource(owned_pod("web-0", "u1", None, "Running")));
    h.script(short());
    let session = h.follow(LogOptions::follow());
    assert!(h.resources.remove(&pod_kind(), Some("default"), "web-0"));
    h.run_for(Duration::from_millis(300));
    let state = session.state();
    assert_eq!(state, LogState::Ended(EndReason::PodDeleted));
    assert!(matches!(state, LogState::Ended(reason) if !reason.has_replacement()));
}

#[test]
fn a_pod_recreated_under_its_name_is_a_replacement() {
    let mut h = Harness::new();
    let owner = Some(("StatefulSet", "web"));
    h.resources
        .insert(resource(owned_pod("web-0", "u1", owner, "Running")));
    h.script(short());
    let session = h.follow(LogOptions::follow());
    h.resources
        .insert(resource(owned_pod("web-0", "u2", owner, "Running")));
    h.run_for(Duration::from_millis(300));
    assert_eq!(session.state(), LogState::Ended(EndReason::PodReplaced));
}

#[test]
fn a_stream_that_closes_while_its_pod_runs_is_reconnected() {
    let mut h = Harness::new();
    h.resources.insert(resource(owned_pod(
        "web-0",
        "u1",
        Some(("ReplicaSet", "web-5d8")),
        "Running",
    )));
    h.script(short());
    h.script(lines(0..3).keep_open());
    let session = h.follow(LogOptions::follow());
    h.run_for(Duration::from_millis(300));
    assert!(
        matches!(session.state(), LogState::Reconnecting { attempt: 1, .. }),
        "{:?}",
        session.state()
    );
    h.run_for(Duration::from_millis(700));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), ["line 0", "line 1", "line 2"]);
}

#[test]
fn a_session_without_pod_reads_ends_unexplained() {
    let mut h = Harness::new();
    h.script(short());
    let session = h.open_plain(LogOptions::follow());
    h.run_for(Duration::from_millis(300));
    assert_eq!(session.state(), LogState::Ended(EndReason::StreamClosed));
    assert!(session.pod_identity().is_none());
}
