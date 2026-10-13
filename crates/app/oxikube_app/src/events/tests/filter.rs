//! Queries: each filter part alone and combined, the regex compile error, newest-first order.

use std::sync::Arc;

use jiff::SignedDuration;
use oxikube_domain::event::EventType;

use super::{ev, t0, uids, warning};
use crate::events::{EventBuffer, EventFilter};

/// a: Normal Pulled, ns-a, pod-1, t=1
/// b: Warning BackOff, ns-a, pod-1, t=5 (reported by kubelet)
/// c: Warning FailedScheduling, ns-b, pod-2, t=3
/// d: Normal ScalingReplicaSet about a Deployment, ns-b, t=4
/// e: Warning FailedMount, ns-c, pod-3, t=2, message mentions "secret-volume"
fn buffer() -> EventBuffer {
    let mut buffer = EventBuffer::new(100);
    buffer.upsert(ev("a", "ns-a", "pod-1", 1));
    let mut b = warning("b", "ns-a", "pod-1", 5, "BackOff");
    b.reporting_component = Some(Arc::from("kubelet"));
    buffer.upsert(b);
    buffer.upsert(warning("c", "ns-b", "pod-2", 3, "FailedScheduling"));
    let mut d = ev("d", "ns-b", "deploy-1", 4);
    d.reason = Arc::from("ScalingReplicaSet");
    d.regarding.gvk.kind = Arc::from("Deployment");
    d.regarding.name = Arc::from("web");
    buffer.upsert(d);
    let mut e = warning("e", "ns-c", "pod-3", 2, "FailedMount");
    e.message = "MountVolume.SetUp failed for volume \"secret-volume\"".into();
    buffer.upsert(e);
    buffer
}

fn run(filter: &EventFilter) -> Vec<String> {
    uids(buffer().query(filter))
}

#[test]
fn everything_comes_newest_first() {
    assert_eq!(run(&EventFilter::new()), ["b", "d", "c", "e", "a"]);
    assert!(EventFilter::new().is_empty());
}

#[test]
fn each_part_alone() {
    assert_eq!(run(&EventFilter::warnings()), ["b", "c", "e"]);
    assert_eq!(
        run(&EventFilter::new().types([EventType::Normal])),
        ["d", "a"]
    );
    assert_eq!(
        run(&EventFilter::new().reasons(["BackOff", "FailedMount"])),
        ["b", "e"]
    );
    assert_eq!(run(&EventFilter::new().kinds(["deployment"])), ["d"]);
    assert_eq!(run(&EventFilter::new().namespaces(["ns-b"])), ["d", "c"]);
    assert_eq!(
        run(&EventFilter::new().namespaces(["ns-a", "ns-c"])),
        ["b", "e", "a"]
    );
    assert_eq!(run(&EventFilter::new().object("pod-1")), ["b", "a"]);
    assert_eq!(
        run(&EventFilter::new().object("nothing")),
        Vec::<String>::new()
    );
    let since = t0() + SignedDuration::from_secs(3);
    assert_eq!(run(&EventFilter::new().since(since)), ["b", "d", "c"]);
}

#[test]
fn text_is_a_case_insensitive_substring_over_reason_message_object_and_reporter() {
    let text = |t: &str| run(&EventFilter::new().text(t).unwrap());
    assert_eq!(text("backoff"), ["b"], "reason");
    assert_eq!(text("SECRET-VOLUME"), ["e"], "message");
    assert_eq!(text("web"), ["d"], "object name");
    assert_eq!(text("KUBELET"), ["b"], "reporting component");
    assert_eq!(text(""), run(&EventFilter::new()), "empty text matches all");
}

#[test]
fn text_with_regex_syntax_is_a_regex_compiled_once() {
    let filter = EventFilter::new()
        .text("^Failed(Mount|Scheduling)$")
        .unwrap();
    assert_eq!(run(&filter), ["c", "e"]);
}

#[test]
fn a_bad_regex_is_an_error_naming_the_pattern() {
    let error = EventFilter::new().text("Failed(").unwrap_err();
    assert_eq!(error.pattern, "Failed(");
    assert!(!error.message.is_empty());
    assert!(!error.message.contains('\n'));
    assert!(error.to_string().contains("Failed("));
}

#[test]
fn parts_combine_with_and() {
    let since = t0() + SignedDuration::from_secs(2);
    let filter = EventFilter::warnings()
        .namespaces(["ns-a", "ns-c"])
        .since(since);
    assert_eq!(run(&filter), ["b", "e"]);
    let filter = EventFilter::new()
        .object("pod-1")
        .types([EventType::Warning])
        .reasons(["BackOff"]);
    assert_eq!(run(&filter), ["b"]);
    let filter = EventFilter::new()
        .kinds(["Pod"])
        .namespaces(["ns-b"])
        .text("sched")
        .unwrap();
    assert_eq!(run(&filter), ["c"]);
}

#[test]
fn a_lazy_query_stops_after_what_is_taken() {
    let buffer = buffer();
    let filter = EventFilter::new();
    assert_eq!(uids(buffer.query(&filter).take(2)), ["b", "d"]);
}
