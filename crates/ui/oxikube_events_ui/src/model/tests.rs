//! The model against coalesced changes: order by last seen, incremental placement agreeing with a
//! fresh sort, the selection following its event, and the paused tail.

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_app::events::{EventKey, EventsChanged};
use oxikube_domain::event::Event;
use oxikube_testkit::event;
use proptest::prelude::*;

use super::*;

fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().unwrap()
}

/// Event `uid`, last seen `secs` after t0, counted `count` times.
fn ev(uid: &str, secs: i64, count: u32) -> Arc<Event> {
    let mut e = event()
        .uid(uid)
        .reason(&format!("R{}", uid.len()))
        .message(&format!("message of {uid}"))
        .build();
    e.last_seen = Some(t0() + SignedDuration::from_secs(secs));
    e.count = count;
    Arc::new(e)
}

/// The events in service order (newest last seen first, later arrival first on a tie).
fn listed(events: &[Arc<Event>]) -> Vec<Arc<Event>> {
    let mut v: Vec<(usize, Arc<Event>)> = events.iter().cloned().enumerate().collect();
    v.sort_by(|(ia, a), (ib, b)| (sort::last_seen(b), ib).cmp(&(sort::last_seen(a), ia)));
    v.into_iter().map(|(_, e)| e).collect()
}

fn model(events: &[Arc<Event>]) -> EventsModel {
    let mut model = EventsModel::new();
    model.install(RowSet::build(listed(events), EventSort::default(), 0));
    model
}

fn uids(model: &EventsModel) -> Vec<String> {
    model
        .events()
        .map(|e| e.uid.as_deref().unwrap().to_owned())
        .collect()
}

fn added(events: &[Arc<Event>]) -> EventsChanged {
    EventsChanged {
        added: events.to_vec(),
        ..EventsChanged::default()
    }
}

#[test]
fn a_list_keeps_the_service_order_newest_first() {
    let m = model(&[ev("a", 1, 1), ev("b", 3, 1), ev("c", 2, 1)]);
    assert_eq!(uids(&m), ["b", "c", "a"]);
}

#[test]
fn new_events_go_on_top_and_updates_move_with_their_count() {
    let mut m = model(&[ev("a", 1, 1), ev("b", 2, 1)]);
    assert!(m.apply(&added(&[ev("c", 3, 1)])));
    assert_eq!(uids(&m), ["c", "b", "a"]);
    // `a` happens again: it moves to the top with its new count.
    let again = EventsChanged {
        updated: vec![ev("a", 4, 5)],
        ..EventsChanged::default()
    };
    assert!(m.apply(&again));
    assert_eq!(uids(&m), ["a", "c", "b"]);
    assert_eq!(m.rows()[0].fixed(EventColumn::Count), "5");
    let gone = EventsChanged {
        removed: vec![EventKey::of(&ev("c", 0, 0))],
        ..EventsChanged::default()
    };
    assert!(m.apply(&gone));
    assert_eq!(uids(&m), ["a", "b"]);
    assert!(!m.apply(&EventsChanged::default()), "nothing to apply");
}

#[test]
fn another_order_places_new_rows_in_it() {
    let mut m = model(&[ev("a", 1, 7), ev("b", 2, 1), ev("c", 3, 4)]);
    m.set_sort(EventSort {
        column: EventColumn::Count,
        descending: true,
    });
    assert_eq!(uids(&m), ["a", "c", "b"]);
    m.apply(&added(&[ev("d", 9, 5)]));
    assert_eq!(uids(&m), ["a", "d", "c", "b"]);
}

#[test]
fn a_list_built_in_the_old_order_is_installed_in_the_new_one() {
    // A read starts in the default order; a header click lands before the read does.
    let events = [ev("a", 1, 7), ev("b", 2, 1), ev("c", 3, 4)];
    let set = RowSet::build(listed(&events), EventSort::default(), 0);
    let mut m = model(&events);
    m.set_sort(EventSort {
        column: EventColumn::Count,
        descending: true,
    });
    m.install(set);
    assert_eq!(uids(&m), ["a", "c", "b"], "the rows follow the header");
    // The binary searches find the rows: an update moves its row, it does not duplicate it.
    let again = EventsChanged {
        updated: vec![ev("b", 4, 9)],
        removed: vec![EventKey::of(&ev("c", 0, 0))],
        ..EventsChanged::default()
    };
    assert!(m.apply(&again));
    assert_eq!(uids(&m), ["b", "a"]);
}

#[test]
fn the_selection_follows_its_event_when_rows_arrive_above_it() {
    let mut m = model(&[ev("a", 1, 1), ev("b", 2, 1)]);
    assert!(m.select(1));
    assert_eq!(m.selected_row().unwrap().event.uid.as_deref(), Some("a"));
    m.apply(&added(&[ev("c", 3, 1), ev("d", 4, 1)]));
    assert_eq!(m.selected_index(), Some(3));
    assert!(m.is_selected(3) && !m.is_selected(0));
    assert_eq!(m.step(-1), Some(2));
    assert_eq!(m.select_end(false), Some(0));
    assert_eq!(m.select_end(true), Some(3));
    // The event goes: so does the selection.
    m.apply(&EventsChanged {
        removed: vec![EventKey::of(&ev("a", 0, 0))],
        ..EventsChanged::default()
    });
    assert_eq!(m.selected_index(), None);
}

#[test]
fn a_paused_tail_counts_new_events_and_resumes_by_reading_again() {
    let mut m = model(&[ev("a", 1, 1)]);
    m.pause(false);
    assert!(
        !m.apply(&added(&[ev("b", 2, 1), ev("c", 3, 1)])),
        "nothing moves"
    );
    assert_eq!(uids(&m), ["a"]);
    assert_eq!(m.unseen(), 2);
    // One of them is gone before the user looked.
    m.apply(&EventsChanged {
        removed: vec![EventKey::of(&ev("b", 0, 0))],
        ..EventsChanged::default()
    });
    assert_eq!(m.unseen(), 1);
    // A scroll does not turn the user's pause into an automatic one.
    m.pause(true);
    assert_eq!(m.tail(), Tail::Paused { auto: false });
    assert!(m.resume(), "read the list again");
    assert_eq!(m.unseen(), 0);
    assert_eq!(m.tail(), Tail::Live);
    assert!(!m.resume(), "nothing changed since");
}

#[test]
fn relative_cells_are_worked_out_once_per_epoch() {
    let mut m = model(&[ev("a", 0, 1)]);
    let now = t0() + SignedDuration::from_secs(180);
    let row = &mut m.rows_mut()[0];
    assert_eq!(row.cell(EventColumn::LastSeen, now, 1).as_ref(), "3m");
    let later = now + SignedDuration::from_secs(120);
    assert_eq!(
        row.cell(EventColumn::LastSeen, later, 1).as_ref(),
        "3m",
        "same epoch: cached"
    );
    assert_eq!(row.cell(EventColumn::LastSeen, later, 2).as_ref(), "5m");
    assert_eq!(row.cell(EventColumn::Reason, later, 2).as_ref(), "R1");
}

#[test]
fn messages_are_one_line_and_capped() {
    assert_eq!(one_line("a\nb\tc", 10), "a b c");
    assert_eq!(one_line("abcdef", 3), "abc…");
}

fn columns() -> impl Strategy<Value = EventSort> {
    (0..EventColumn::ALL.len(), any::<bool>()).prop_map(|(ix, descending)| EventSort {
        column: EventColumn::ALL[ix],
        descending,
    })
}

proptest! {
    /// Applying changes one by one (or by sorting a burst) gives the same rows as building the
    /// list from scratch in that order.
    #[test]
    fn incremental_placement_agrees_with_a_fresh_sort(
        sort in columns(),
        initial in prop::collection::vec((0u8..40, 0i64..50, 1u32..9), 0..60),
        batches in prop::collection::vec(
            prop::collection::vec((0u8..40, 0i64..50, 1u32..9, any::<bool>()), 0..300),
            1..4,
        ),
    ) {
        let mk = |n: u8, secs: i64, count: u32| ev(&format!("e{n}"), secs, count);
        let mut truth: std::collections::BTreeMap<u8, Arc<Event>> = initial
            .iter()
            .map(|&(n, s, c)| (n, mk(n, s, c)))
            .collect();
        let start: Vec<Arc<Event>> = truth.values().cloned().collect();
        let mut m = model(&start);
        m.set_sort(sort);
        for batch in batches {
            let mut changed = EventsChanged::default();
            let mut seen = std::collections::HashSet::new();
            for (n, secs, count, remove) in batch {
                if !seen.insert(n) {
                    continue;
                }
                if remove {
                    if let Some(old) = truth.remove(&n) {
                        changed.removed.push(EventKey::of(&old));
                    }
                } else {
                    let e = mk(n, secs, count);
                    if truth.insert(n, e.clone()).is_some() {
                        changed.updated.push(e);
                    } else {
                        changed.added.push(e);
                    }
                }
            }
            m.apply(&changed);
        }
        let got: Vec<Arc<Event>> = m.events().cloned().collect();
        prop_assert_eq!(got.len(), truth.len());
        // Same set, and in order by the column (ties may differ in arrival).
        for pair in m.rows().windows(2) {
            prop_assert!(sort.compare(&pair[0], &pair[1]).is_lt());
        }
        for e in truth.values() {
            prop_assert!(got.iter().any(|g| g == e));
        }
    }
}
