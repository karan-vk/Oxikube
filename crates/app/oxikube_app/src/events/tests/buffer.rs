//! The ring: dedupe by uid, eviction at capacity, capacity changes, deletes and relists, with
//! every index checked after each step.

use std::sync::Arc;

use jiff::SignedDuration;
use oxikube_ports::Delta;

use super::{ev, t0, uids, warning};
use crate::events::{EventBuffer, EventFilter, EventKey, Upserted};

#[test]
fn a_second_delivery_updates_count_last_seen_and_message_in_place() {
    let mut buffer = EventBuffer::new(10);
    assert_eq!(buffer.upsert(ev("a", "ns", "p1", 1)), Upserted::Added);
    assert_eq!(buffer.upsert(ev("b", "ns", "p2", 2)), Upserted::Added);
    let mut again = ev("a", "ns", "p1", 5);
    again.count = 7;
    again.message = "pulled again".into();
    assert_eq!(buffer.upsert(again.clone()), Upserted::Updated);
    assert_eq!(buffer.upsert(again), Upserted::Unchanged);
    buffer.assert_consistent();

    assert_eq!(buffer.len(), 2);
    let a = buffer.get(&EventKey::of(&ev("a", "ns", "p1", 0))).unwrap();
    assert_eq!(a.count, 7);
    assert_eq!(a.message, "pulled again");
    assert_eq!(a.last_seen, Some(t0() + SignedDuration::from_secs(5)));
    // It moved to the recent end.
    assert_eq!(uids(buffer.query(&EventFilter::new())), ["a", "b"]);
}

#[test]
fn at_capacity_the_oldest_by_last_seen_is_evicted_from_every_index() {
    let mut buffer = EventBuffer::new(3);
    buffer.upsert(warning("w-old", "ns-a", "pod-x", 1, "BackOff"));
    buffer.upsert(ev("b", "ns-b", "pod-y", 3));
    buffer.upsert(ev("c", "ns-b", "pod-y", 2));
    buffer.upsert(ev("d", "ns-c", "pod-z", 4));
    buffer.assert_consistent();

    assert_eq!(buffer.len(), 3);
    assert_eq!(buffer.evicted(), 1);
    assert_eq!(uids(buffer.query(&EventFilter::new())), ["d", "b", "c"]);
    assert_eq!(
        buffer.object_counts("pod-x"),
        (0, 0),
        "no stale object entry"
    );
    assert_eq!(buffer.warnings().count(), 0, "no stale warning entry");
    assert!(
        buffer
            .query(&EventFilter::new().namespaces(["ns-a"]))
            .next()
            .is_none()
    );
}

#[test]
fn an_event_older_than_everything_in_a_full_buffer_is_not_stored() {
    let mut buffer = EventBuffer::new(2);
    buffer.upsert(ev("a", "ns", "p", 10));
    buffer.upsert(ev("b", "ns", "p", 11));
    assert_eq!(buffer.upsert(ev("old", "ns", "p", 1)), Upserted::Dropped);
    assert_eq!(uids(buffer.query(&EventFilter::new())), ["b", "a"]);
    assert_eq!(buffer.evicted(), 1);
    buffer.assert_consistent();
}

#[test]
fn shrinking_evicts_the_oldest_and_growing_keeps_everything() {
    let mut buffer = EventBuffer::new(5);
    for i in 0..5 {
        buffer.upsert(ev(&format!("e{i}"), "ns", "p", i));
    }
    buffer.set_capacity(2);
    buffer.assert_consistent();
    assert_eq!(uids(buffer.query(&EventFilter::new())), ["e4", "e3"]);
    buffer.set_capacity(10);
    assert_eq!(buffer.len(), 2);
    for i in 5..9 {
        buffer.upsert(ev(&format!("e{i}"), "ns", "p", i));
    }
    assert_eq!(buffer.len(), 6);
    buffer.assert_consistent();
}

#[test]
fn deletes_and_relists_keep_the_indexes_clean() {
    let mut buffer = EventBuffer::new(10);
    buffer.apply(Delta::Applied(
        warning("w", "ns", "pod", 1, "BackOff").into(),
    ));
    buffer.apply(Delta::Applied(ev("n", "ns", "pod", 2).into()));
    assert_eq!(buffer.object_counts("pod"), (2, 1));
    buffer.apply(Delta::Deleted(
        warning("w", "ns", "pod", 1, "BackOff").into(),
    ));
    assert_eq!(buffer.object_counts("pod"), (1, 0));
    buffer.assert_consistent();

    buffer.apply(Delta::Restarted(vec![
        ev("x", "other", "pod-2", 5).into(),
        ev("x", "other", "pod-2", 6).into(),
    ]));
    buffer.assert_consistent();
    assert_eq!(buffer.len(), 1, "a relist replaces everything and dedupes");
    assert_eq!(buffer.object_counts("pod"), (0, 0));
    let all = EventFilter::new();
    let x = buffer.query(&all).next().unwrap();
    assert_eq!(x.last_seen, Some(t0() + SignedDuration::from_secs(6)));
}

#[test]
fn events_without_a_uid_dedupe_on_object_reason_and_message() {
    let mut buffer = EventBuffer::new(10);
    let mut first = ev("x", "ns", "pod", 1);
    first.uid = None;
    let mut again = first.clone();
    again.count = 3;
    again.last_seen = Some(t0() + SignedDuration::from_secs(9));
    buffer.upsert(first);
    assert_eq!(buffer.upsert(again), Upserted::Updated);
    assert_eq!(buffer.len(), 1);
    assert_eq!(
        buffer.query(&EventFilter::new()).next().map(|e| e.count),
        Some(3)
    );
    buffer.assert_consistent();
}

#[test]
fn from_events_keeps_the_newest_capacity() {
    let events = (0..10).map(|i| ev(&format!("e{i}"), "ns", "p", 10 - i));
    let buffer = EventBuffer::from_events(3, events);
    assert_eq!(uids(buffer.query(&EventFilter::new())), ["e0", "e1", "e2"]);
    let held: Vec<Arc<_>> = buffer.ring_events();
    assert_eq!(held.len(), 3);
    buffer.assert_consistent();
}

#[test]
fn the_buffer_keeps_the_feeds_allocation_instead_of_a_copy() {
    // The adapter's ring holds the same `Arc` it put in the delta: storing a copy here would
    // hold every event twice per cluster (the E14 audit's 35 MB for a 10k buffer).
    let mut buffer = EventBuffer::new(10);
    let applied = std::sync::Arc::new(ev("a", "ns", "pod", 1));
    buffer.apply(Delta::Applied(applied.clone()));
    let listed = std::sync::Arc::new(ev("b", "ns", "pod", 2));
    let relisted = EventBuffer::from_events(10, [listed.clone()]);
    let all = EventFilter::new();
    let held = buffer.query(&all).next().unwrap();
    assert!(std::sync::Arc::ptr_eq(held, &applied), "applied: shared");
    let held = relisted.query(&all).next().unwrap();
    assert!(std::sync::Arc::ptr_eq(held, &listed), "relisted: shared");
}
