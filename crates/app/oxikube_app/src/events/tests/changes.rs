//! Coalescing: a burst of buffer changes becomes one `EventsChanged` per poll, filtered per
//! subscriber.

use oxikube_domain::event::EventType;

use super::{ev, warning};
use crate::events::changes::Pending;
use crate::events::{EventBuffer, EventFilter, EventKey};

fn changes_of(
    buffer: &mut EventBuffer,
    events: Vec<oxikube_domain::event::Event>,
) -> Vec<crate::events::changes::Change> {
    let mut out = Vec::new();
    for e in events {
        buffer.upsert_into(e, &mut out);
    }
    out
}

#[test]
fn a_burst_merges_per_event() {
    let mut buffer = EventBuffer::new(10);
    let mut pending = Pending::default();
    let mut again = ev("a", "ns", "p", 2);
    again.count = 2;
    let changes = changes_of(
        &mut buffer,
        vec![ev("a", "ns", "p", 1), again, ev("b", "ns", "p", 3)],
    );
    pending.push(&changes, &EventFilter::new());
    let out = pending.take();
    assert!(!out.reset);
    assert_eq!(out.added.len(), 2, "added then updated is one addition");
    assert_eq!(out.added[0].count, 2, "in its newest state");
    assert!(out.updated.is_empty() && out.removed.is_empty());
    assert!(pending.is_empty());
}

#[test]
fn added_then_removed_before_a_poll_is_nothing() {
    let mut buffer = EventBuffer::new(1);
    let mut pending = Pending::default();
    let changes = changes_of(
        &mut buffer,
        vec![ev("a", "ns", "p", 1), ev("b", "ns", "p", 2)],
    );
    pending.push(&changes, &EventFilter::new());
    let out = pending.take();
    assert_eq!(super::uids(&out.added), ["b"]);
    assert!(out.removed.is_empty());
}

#[test]
fn updates_crossing_the_filter_become_additions_and_removals() {
    let mut buffer = EventBuffer::new(10);
    let warnings = EventFilter::warnings();
    let mut pending = Pending::default();
    changes_of(
        &mut buffer,
        vec![ev("a", "ns", "p", 1), warning("b", "ns", "p", 1, "BackOff")],
    );
    // `a` turns into a warning, `b` into a normal event.
    let mut a = ev("a", "ns", "p", 2);
    a.event_type = EventType::Warning;
    let b = ev("b", "ns", "p", 2);
    let changes = changes_of(&mut buffer, vec![a, b]);
    pending.push(&changes, &warnings);
    let out = pending.take();
    assert_eq!(super::uids(&out.added), ["a"]);
    assert_eq!(out.removed, [EventKey::of(&ev("b", "x", "x", 0))]);
}

#[test]
fn a_reset_swallows_the_changes_and_too_many_become_a_reset() {
    let mut buffer = EventBuffer::new(10_000);
    let mut pending = Pending::reset();
    let changes = changes_of(&mut buffer, vec![ev("a", "ns", "p", 1)]);
    pending.push(&changes, &EventFilter::new());
    let out = pending.take();
    assert!(out.reset && out.added.is_empty());

    let burst = (0..5_000)
        .map(|i| ev(&format!("e{i}"), "ns", "p", i))
        .collect();
    let changes = changes_of(&mut buffer, burst);
    pending.push(&changes, &EventFilter::new());
    let out = pending.take();
    assert!(out.reset, "a burst past the pending bound is a reset");
    assert!(out.added.is_empty());
}
