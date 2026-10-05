//! The ring: eviction order and counter, de-duplication across APIs, per-namespace relists.

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_domain::event::Event;
use oxikube_ports::Delta;

use super::*;
use crate::events::config::EventApi::{Core, EventsV1};
use crate::events::ring::{EventRing, Key};

const NONE: Option<Arc<str>> = None;

fn event(uid: &str, last: &str, count: u32) -> Event {
    domain(&core_event(uid, "web-0", last, count))
}

/// `+uid@count` / `-uid@count` per delta.
fn labels(deltas: &[Delta<Event>]) -> Vec<String> {
    deltas
        .iter()
        .map(|d| match d {
            Delta::Applied(e) => format!("+{}@{}", e.uid.as_deref().unwrap(), e.count),
            Delta::Deleted(e) => format!("-{}@{}", e.uid.as_deref().unwrap(), e.count),
            Delta::Restarted(_) => "restart".into(),
        })
        .collect()
}

fn put(ring: &mut EventRing, api: crate::events::EventApi, e: Event) -> Vec<String> {
    let mut out = Vec::new();
    ring.upsert(Key::of(&e), api, &NONE, e, &mut out);
    labels(&out)
}

#[test]
fn the_oldest_by_last_seen_is_evicted_first_and_counted() {
    let mut ring = EventRing::new(3);
    // Arrival order differs from last-seen order.
    assert_eq!(
        put(&mut ring, Core, event("b", "2026-10-03T11:02:00Z", 1)),
        ["+b@1"]
    );
    assert_eq!(
        put(&mut ring, Core, event("a", "2026-10-03T11:01:00Z", 1)),
        ["+a@1"]
    );
    assert_eq!(
        put(&mut ring, Core, event("c", "2026-10-03T11:03:00Z", 1)),
        ["+c@1"]
    );
    assert_eq!(ring.evicted(), 0);

    // Full: the newcomer evicts `a` (oldest last-seen), the eviction is sent first.
    assert_eq!(
        put(&mut ring, Core, event("d", "2026-10-03T11:04:00Z", 1)),
        ["-a@1", "+d@1"]
    );
    assert_eq!(
        put(&mut ring, Core, event("e", "2026-10-03T11:05:00Z", 1)),
        ["-b@1", "+e@1"]
    );
    assert_eq!(ring.len(), 3);
    assert_eq!(ring.evicted(), 2);
    let held: Vec<_> = ring
        .snapshot()
        .iter()
        .map(|e| e.uid.clone().unwrap())
        .collect();
    assert_eq!(held, ["c".into(), "d".into(), "e".into()] as [Arc<str>; 3]);
}

#[test]
fn an_event_older_than_everything_held_is_dropped_but_counted() {
    let mut ring = EventRing::new(2);
    put(&mut ring, Core, event("a", "2026-10-03T11:05:00Z", 1));
    put(&mut ring, Core, event("b", "2026-10-03T11:06:00Z", 1));
    assert!(put(&mut ring, Core, event("old", "2026-10-03T10:00:00Z", 1)).is_empty());
    assert_eq!((ring.len(), ring.evicted()), (2, 1));
}

#[test]
fn a_shed_event_redelivered_by_the_other_api_or_a_relist_is_counted_once() {
    let mut ring = EventRing::new(3);
    let all = [
        ("e1", "2026-10-03T11:01:00Z"),
        ("e2", "2026-10-03T11:02:00Z"),
        ("e3", "2026-10-03T11:03:00Z"),
        ("e4", "2026-10-03T11:04:00Z"),
    ];
    for (uid, at) in all {
        put(&mut ring, Core, event(uid, at, 1));
    }
    assert_eq!(ring.evicted(), 1, "e1");
    // The other API's list, then a relist of each: e1 is older than everything held.
    for api in [EventsV1, Core, EventsV1] {
        for (uid, at) in all {
            put(&mut ring, api, event(uid, at, 1));
        }
    }
    assert_eq!((ring.len(), ring.evicted()), (3, 1));
}

#[test]
fn a_shed_event_tied_with_the_oldest_does_not_push_out_a_live_one() {
    let mut ring = EventRing::new(2);
    put(&mut ring, Core, event("a", "2026-10-03T11:01:00Z", 1));
    put(&mut ring, Core, event("b", "2026-10-03T11:01:00Z", 1));
    // Same second as `a` and `b`: `c` evicts `a` as the oldest by arrival.
    assert_eq!(
        put(&mut ring, Core, event("c", "2026-10-03T11:01:00Z", 1)),
        ["-a@1", "+c@1"]
    );
    // The other API's late view of `a` ties the oldest held: dropped, no churn, one count.
    assert!(put(&mut ring, EventsV1, event("a", "2026-10-03T11:01:00Z", 1)).is_empty());
    assert_eq!((ring.len(), ring.evicted()), (2, 1));
}

#[test]
fn a_shed_event_that_comes_back_newer_or_is_deleted_stops_counting() {
    let mut ring = EventRing::new(2);
    put(&mut ring, Core, event("a", "2026-10-03T11:01:00Z", 1));
    put(&mut ring, Core, event("b", "2026-10-03T11:02:00Z", 1));
    put(&mut ring, Core, event("c", "2026-10-03T11:03:00Z", 1));
    put(&mut ring, Core, event("old", "2026-10-03T10:00:00Z", 1));
    assert_eq!(ring.evicted(), 2, "a and old");

    // `old` is deleted on the server: nothing is hidden for it any more.
    let old = event("old", "2026-10-03T10:00:00Z", 1);
    ring.remove(&Key::of(&old), &mut Vec::new());
    assert_eq!(ring.evicted(), 1);

    // `a` happens again and is newer than everything: it is held, `b` is shed instead.
    assert_eq!(
        put(&mut ring, Core, event("a", "2026-10-03T11:04:00Z", 2)),
        ["-b@1", "+a@2"]
    );
    assert_eq!((ring.len(), ring.evicted()), (2, 1), "b only");
}

#[test]
fn an_updated_event_moves_to_the_newest_end() {
    let mut ring = EventRing::new(2);
    put(&mut ring, Core, event("a", "2026-10-03T11:01:00Z", 1));
    put(&mut ring, Core, event("b", "2026-10-03T11:02:00Z", 1));
    // `a` happens again, so `b` is now the oldest.
    assert_eq!(
        put(&mut ring, Core, event("a", "2026-10-03T11:03:00Z", 2)),
        ["+a@2"]
    );
    assert_eq!(
        put(&mut ring, Core, event("c", "2026-10-03T11:04:00Z", 1)),
        ["-b@1", "+c@1"]
    );
}

#[test]
fn an_event_without_a_time_is_the_oldest() {
    let mut ring = EventRing::new(2);
    let mut untimed = core_event("u", "web-0", "2026-10-03T11:00:00Z", 1);
    untimed.as_object_mut().unwrap().remove("lastTimestamp");
    untimed.as_object_mut().unwrap().remove("firstTimestamp");
    untimed["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("creationTimestamp");
    let untimed = domain(&untimed);
    assert!(untimed.last_seen.is_none());
    put(&mut ring, Core, untimed);
    put(&mut ring, Core, event("a", "2026-10-03T11:01:00Z", 1));
    assert_eq!(
        put(&mut ring, Core, event("b", "2026-10-03T11:02:00Z", 1)),
        ["-u@1", "+b@1"]
    );
}

#[test]
fn both_views_of_one_event_make_one_entry_and_one_delta() {
    let mut ring = EventRing::new(10);
    let core = domain(&core_event("x", "web-0", "2026-10-03T11:05:00Z", 3));
    let v1 = domain(&v1_event("x", "web-0", "2026-10-03T11:05:00Z", 3));
    assert_eq!(put(&mut ring, Core, core.clone()), ["+x@3"]);
    // The other API's view of the same stored object: same uid, same time and count.
    assert!(put(&mut ring, EventsV1, v1).is_empty());
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.snapshot(), std::slice::from_ref(&core));
    // And again, the same view twice (a resync): nothing.
    assert!(put(&mut ring, Core, core).is_empty());
}

#[test]
fn a_strictly_newer_view_from_either_api_wins_and_an_older_one_loses() {
    let mut ring = EventRing::new(10);
    put(
        &mut ring,
        Core,
        domain(&core_event("x", "web-0", "2026-10-03T11:05:00Z", 3)),
    );
    // events.k8s.io is ahead by one occurrence.
    assert_eq!(
        put(
            &mut ring,
            EventsV1,
            domain(&v1_event("x", "web-0", "2026-10-03T11:06:00Z", 4))
        ),
        ["+x@4"]
    );
    // core/v1 catches up with the same update: the tie keeps the entry, no delta.
    assert!(
        put(
            &mut ring,
            Core,
            domain(&core_event("x", "web-0", "2026-10-03T11:06:00Z", 4))
        )
        .is_empty()
    );
    // A stale view from the other API does not roll it back.
    assert!(
        put(
            &mut ring,
            Core,
            domain(&core_event("x", "web-0", "2026-10-03T11:05:00Z", 3))
        )
        .is_empty()
    );
    assert_eq!(ring.snapshot()[0].count, 4);
}

#[test]
fn an_event_without_a_uid_is_keyed_on_object_reason_and_message() {
    let strip = |mut v: Value| {
        v["metadata"].as_object_mut().unwrap().remove("uid");
        domain(&v)
    };
    let mut ring = EventRing::new(10);
    let a = strip(core_event("ignored", "web-0", "2026-10-03T11:05:00Z", 1));
    let a_again = strip(v1_event("ignored", "web-0", "2026-10-03T11:05:00Z", 1));
    let other = strip(core_event("ignored", "web-1", "2026-10-03T11:05:00Z", 1));
    assert_eq!(Key::of(&a), Key::of(&a_again));
    assert_ne!(Key::of(&a), Key::of(&other));
    let mut out = Vec::new();
    ring.upsert(Key::of(&a), Core, &NONE, a, &mut out);
    ring.upsert(Key::of(&a_again), EventsV1, &NONE, a_again, &mut out);
    ring.upsert(Key::of(&other), Core, &NONE, other, &mut out);
    assert_eq!(ring.len(), 2);
    assert_eq!(out.len(), 2, "the duplicate produced no delta");
}

#[test]
fn remove_and_retire_unseen() {
    let ns = |s: &str| Some(Arc::<str>::from(s));
    let mut ring = EventRing::new(10);
    let mut out = Vec::new();
    for (uid, namespace) in [("a1", ns("a")), ("a2", ns("a")), ("b1", ns("b"))] {
        let e = event(uid, "2026-10-03T11:05:00Z", 1);
        ring.upsert(Key::of(&e), Core, &namespace, e, &mut out);
    }
    out.clear();

    // A relist of namespace `a` that returns only a1: a2 is retired, b1 is not a's business.
    let seen: HashSet<Key> = [Key::Uid("a1".into())].into();
    ring.retire_unseen(Core, &ns("a"), &seen, &mut out);
    assert_eq!(labels(&out), ["-a2@1"]);
    assert_eq!(ring.len(), 2);

    out.clear();
    ring.remove(&Key::Uid("b1".into()), &mut out);
    ring.remove(&Key::Uid("b1".into()), &mut out);
    assert_eq!(
        labels(&out),
        ["-b1@1"],
        "removing a missing event does nothing"
    );
    assert_eq!(ring.snapshot().len(), 1);
}

#[test]
fn a_relist_of_one_api_does_not_retire_what_the_other_delivered() {
    let mut ring = EventRing::new(10);
    let mut out = Vec::new();
    let both = event("both", "2026-10-03T11:01:00Z", 1);
    let only_core = event("only-core", "2026-10-03T11:02:00Z", 1);
    ring.upsert(Key::of(&both), Core, &NONE, both.clone(), &mut out);
    ring.upsert(Key::of(&both), EventsV1, &NONE, both, &mut out);
    ring.upsert(Key::of(&only_core), Core, &NONE, only_core, &mut out);
    out.clear();

    // events.k8s.io relists with nothing: `both` stays (core still holds it), and
    // `only-core` was never its business.
    ring.retire_unseen(EventsV1, &NONE, &HashSet::new(), &mut out);
    assert!(out.is_empty());
    // core relists and returns `both` only: `only-core` goes, `both` stays.
    let seen: HashSet<Key> = [Key::Uid("both".into())].into();
    ring.retire_unseen(Core, &NONE, &seen, &mut out);
    assert_eq!(labels(&out), ["-only-core@1"]);
    assert_eq!(ring.len(), 1);
}
