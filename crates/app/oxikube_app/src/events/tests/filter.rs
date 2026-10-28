//! Queries: each filter part alone and combined, the regex compile error, newest-first order.

use std::sync::Arc;

use jiff::SignedDuration;
use oxikube_domain::event::EventType;

use super::{ev, t0, uids, warning};
use crate::events::{EventBuffer, EventFilter, ObjectIdentity, ObjectTarget};

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

/// The pod `pod-1` of `ns-a` with the uid `uid`: what a row or a tab looks up.
fn pod_target(uid: &str) -> ObjectTarget {
    ObjectTarget::named(
        Some(Arc::from(uid)),
        ObjectIdentity::new("Pod", Some("ns-a"), "pod-1"),
    )
}

#[test]
fn an_event_whose_writer_gave_no_uid_is_about_its_object_by_name() {
    let mut buffer = EventBuffer::new(10);
    buffer.upsert(ev("with-uid", "ns-a", "pod-1", 1));
    let mut nameless = ev("no-uid", "ns-a", "pod-1", 5);
    nameless.regarding_uid = None;
    buffer.upsert(nameless);
    buffer.assert_consistent();

    let filter = EventFilter::new().object_target(pod_target("pod-1"));
    assert_eq!(
        uids(buffer.query(&filter)),
        ["no-uid", "with-uid"],
        "the event that gave no uid is listed too"
    );
}

#[test]
fn an_event_with_another_objects_uid_is_not_about_this_one() {
    let mut buffer = EventBuffer::new(10);
    // The pod was deleted and re-created under the same name: the old pod's event carries the
    // old uid.
    let mut old = ev("old", "ns-a", "pod-1", 1);
    old.regarding_uid = Some(Arc::from("pod-1-old"));
    buffer.upsert(old);
    let mut nameless = ev("no-uid", "ns-a", "pod-1", 5);
    nameless.regarding_uid = None;
    buffer.upsert(nameless);
    buffer.assert_consistent();

    let filter = EventFilter::new().object_target(pod_target("pod-1-new"));
    assert_eq!(
        uids(buffer.query(&filter)),
        ["no-uid"],
        "the old pod's event stays with the old pod"
    );
}

#[test]
fn a_query_by_uid_alone_ignores_the_events_that_gave_no_uid() {
    let mut buffer = EventBuffer::new(10);
    let mut nameless = ev("no-uid", "ns-a", "pod-1", 5);
    nameless.regarding_uid = None;
    buffer.upsert(nameless);
    // Without the name there is nothing to match the event on.
    let filter = EventFilter::new().object("pod-1");
    assert!(buffer.query(&filter).next().is_none());
}

#[test]
fn a_query_by_name_alone_finds_the_events_that_gave_no_uid() {
    let mut buffer = EventBuffer::new(10);
    let mut nameless = ev("no-uid", "ns-a", "pod-1", 5);
    nameless.regarding_uid = None;
    buffer.upsert(nameless);
    let target = ObjectTarget::named(None, ObjectIdentity::new("Pod", Some("ns-a"), "pod-1"));
    let filter = EventFilter::new().object_target(target);
    assert_eq!(uids(buffer.query(&filter)), ["no-uid"]);
}

#[test]
fn the_two_indexes_of_an_object_are_merged_newest_first() {
    let mut buffer = EventBuffer::new(10);
    // Two events carry the object's uid, two gave none: the list is one run, newest first.
    buffer.upsert(ev("uid-old", "ns-a", "pod-1", 1));
    let mut nameless_old = ev("no-uid-old", "ns-a", "pod-1", 3);
    nameless_old.regarding_uid = None;
    buffer.upsert(nameless_old);
    buffer.upsert(ev("uid-new", "ns-a", "pod-1", 4));
    let mut nameless_new = ev("no-uid-new", "ns-a", "pod-1", 7);
    nameless_new.regarding_uid = None;
    buffer.upsert(nameless_new);
    buffer.assert_consistent();

    let filter = EventFilter::new().object_target(pod_target("pod-1"));
    assert_eq!(
        uids(buffer.query(&filter)),
        ["no-uid-new", "uid-new", "no-uid-old", "uid-old"]
    );
}

#[test]
fn the_events_of_an_object_are_counted_by_uid_and_by_name() {
    let mut buffer = EventBuffer::new(10);
    buffer.upsert(ev("with-uid", "ns-a", "pod-1", 1));
    let mut nameless = warning("no-uid", "ns-a", "pod-1", 5, "BackOff");
    nameless.regarding_uid = None;
    buffer.upsert(nameless);
    let identity = ObjectIdentity::new("Pod", Some("ns-a"), "pod-1");
    assert_eq!(buffer.object_counts("pod-1", Some(&identity)), (2, 1));
    assert_eq!(buffer.object_counts("pod-1", None), (1, 0));
}
