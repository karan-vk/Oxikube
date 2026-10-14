//! The tracker keeps the backlog quiet and tells only what happens while it watches.

use std::sync::Arc;

use super::{t0, warning};
use crate::events::EventKey;
use crate::notifications::tracker::{CLOCK_ALLOWANCE, Tracker};

#[test]
fn the_backlog_of_the_first_list_is_not_news() {
    let mut tracker = Tracker::new(t0());
    let backlog = [
        Arc::new(warning("old", "web-0", "BackOff", 9, -3_600)),
        Arc::new(warning("older", "web-1", "BackOff", 1, -7_200)),
    ];
    assert!(tracker.reset(&backlog).is_empty());
    assert_eq!(tracker.len(), 2);
}

#[test]
fn the_first_list_is_backlog_even_when_recent() {
    let mut tracker = Tracker::new(t0());
    let recent = [Arc::new(warning("now", "web-0", "BackOff", 1, 0))];
    assert!(tracker.reset(&recent).is_empty());
    // Then its count rises: that is news.
    assert_eq!(
        tracker.observe(&warning("now", "web-0", "BackOff", 2, 3), false),
        Some(1)
    );
}

#[test]
fn an_addition_after_the_start_is_news_and_a_count_rise_is_news_by_the_difference() {
    let mut tracker = Tracker::new(t0());
    assert_eq!(
        tracker.observe(&warning("e", "web-0", "BackOff", 1, 5), false),
        Some(1)
    );
    assert_eq!(
        tracker.observe(&warning("e", "web-0", "BackOff", 4, 9), false),
        Some(3)
    );
    assert_eq!(
        tracker.observe(&warning("e", "web-0", "BackOff", 4, 9), false),
        None,
        "no rise"
    );
}

#[test]
fn an_old_count_rising_is_news_even_from_the_backlog() {
    let mut tracker = Tracker::new(t0());
    tracker.reset(&[Arc::new(warning("e", "web-0", "BackOff", 9, -60))]);
    assert_eq!(
        tracker.observe(&warning("e", "web-0", "BackOff", 10, 1), false),
        Some(1)
    );
}

#[test]
fn an_addition_within_the_clock_allowance_counts_and_an_older_one_does_not() {
    let mut tracker = Tracker::new(t0());
    let allowance = CLOCK_ALLOWANCE.as_secs();
    assert_eq!(
        tracker.observe(&warning("a", "web-0", "BackOff", 1, 1 - allowance), false),
        Some(1)
    );
    assert_eq!(
        tracker.observe(&warning("b", "web-1", "BackOff", 1, -allowance - 1), false),
        None
    );
}

#[test]
fn a_relist_after_a_reconnect_tells_only_the_gap() {
    let mut tracker = Tracker::new(t0());
    let before = Arc::new(warning("a", "web-0", "BackOff", 2, 10));
    assert_eq!(tracker.observe(&before, false), Some(1));
    // The disconnect: the feed stops and the buffer empties. Nothing changes.
    assert!(tracker.reset(&[]).is_empty());
    // The relist: the old event as it was, one that rose, and one that happened in the gap.
    let relist = [
        before.clone(),
        Arc::new(warning("b", "web-1", "Failed", 1, 30)),
        Arc::new(warning("c", "web-2", "BackOff", 1, 5)),
    ];
    let news: Vec<_> = tracker
        .reset(&relist)
        .into_iter()
        .map(|(e, n)| (e.uid.as_deref().unwrap().to_owned(), n))
        .collect();
    assert_eq!(
        news,
        [("b".to_owned(), 1)],
        "c is older than what was already told"
    );
}

#[test]
fn a_reset_trims_the_counts_to_what_the_buffer_holds() {
    let mut tracker = Tracker::new(t0());
    tracker.observe(&warning("a", "web-0", "BackOff", 1, 1), false);
    tracker.observe(&warning("b", "web-1", "BackOff", 1, 2), false);
    tracker.reset(&[Arc::new(warning("b", "web-1", "BackOff", 1, 2))]);
    assert_eq!(tracker.len(), 1);
    tracker.forget(&EventKey::of(&warning("b", "web-1", "BackOff", 1, 2)));
    assert_eq!(tracker.len(), 0);
}
