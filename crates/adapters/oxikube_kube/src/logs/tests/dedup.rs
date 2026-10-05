//! Replay dedup: what a reconnect drops and what it must keep.

use crate::logs::dedup::Dedup;

use super::fake::ts;

/// Delivers lines `(n, key)` and returns the ones admitted.
fn feed(dedup: &mut Dedup, lines: &[(i64, u64)]) -> Vec<(i64, u64)> {
    lines
        .iter()
        .copied()
        .filter(|(n, key)| dedup.admit(ts(*n), *key))
        .collect()
}

#[test]
fn nothing_is_dropped_outside_a_replay() {
    let mut d = Dedup::new(100);
    // The same (timestamp, text) twice in a normal stream: both are real lines.
    assert_eq!(feed(&mut d, &[(1, 7), (1, 7), (2, 7)]).len(), 3);
}

#[test]
fn replayed_lines_are_dropped_and_new_ones_kept() {
    let mut d = Dedup::new(100);
    feed(&mut d, &[(1, 1), (2, 2), (3, 3)]);
    d.begin_replay();
    // The overlap replays 2 and 3, then 4 and 5 are new.
    assert_eq!(
        feed(&mut d, &[(2, 2), (3, 3), (4, 4), (5, 5)]),
        [(4, 4), (5, 5)]
    );
    assert_eq!(d.newest(), Some(ts(5)));
}

#[test]
fn the_same_text_at_a_new_time_is_not_a_replay() {
    let mut d = Dedup::new(100);
    feed(&mut d, &[(1, 9), (2, 9)]);
    d.begin_replay();
    // Replay of both, then the app logs the same text again at t=3: kept.
    assert_eq!(feed(&mut d, &[(1, 9), (2, 9), (3, 9)]), [(3, 9)]);
}

#[test]
fn identical_lines_with_one_timestamp_are_matched_as_a_multiset() {
    let mut d = Dedup::new(100);
    feed(&mut d, &[(1, 5), (1, 5)]);
    d.begin_replay();
    // Both copies replay: both dropped. A third copy is genuinely new.
    assert_eq!(feed(&mut d, &[(1, 5), (1, 5), (1, 5)]), [(1, 5)]);
}

#[test]
fn a_line_missed_in_the_gap_is_delivered_even_if_older_than_newest() {
    let mut d = Dedup::new(100);
    feed(&mut d, &[(1, 1), (3, 3)]);
    d.begin_replay();
    // Line 2 was written while disconnected (a restart's previous instance, say).
    assert_eq!(
        feed(&mut d, &[(1, 1), (2, 2), (3, 3), (4, 4)]),
        [(2, 2), (4, 4)]
    );
}

#[test]
fn a_replay_older_than_the_first_line_delivered_is_not_a_gap_fill() {
    let mut d = Dedup::new(100);
    // A `tail` window: the caller only wanted from line 5 on.
    feed(&mut d, &[(5, 5), (6, 6)]);
    d.begin_replay();
    // The overlap replays 1..4 as well, which the window excluded.
    assert_eq!(
        feed(
            &mut d,
            &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 7)]
        ),
        [(7, 7)]
    );
}

#[test]
fn the_replay_ends_at_the_first_newer_line() {
    let mut d = Dedup::new(100);
    feed(&mut d, &[(1, 1)]);
    d.begin_replay();
    assert_eq!(feed(&mut d, &[(2, 2)]), [(2, 2)]);
    // Not a replay any more: a repeat of a delivered key is a real line.
    assert_eq!(feed(&mut d, &[(2, 2)]), [(2, 2)]);
}

#[test]
fn memory_is_bounded_by_the_window() {
    let mut d = Dedup::new(3);
    feed(&mut d, &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    d.begin_replay();
    // 3, 4 and 5 are inside the window and recognised.
    assert_eq!(feed(&mut d, &[(3, 3), (4, 4), (5, 5)]), []);

    let mut d = Dedup::new(3);
    feed(&mut d, &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    d.begin_replay();
    // 1 fell out of the window, so its replay is delivered (the documented limit of
    // `LogsConfig::dedup_window`).
    assert_eq!(feed(&mut d, &[(1, 1)]), [(1, 1)]);
}
