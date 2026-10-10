//! `MatchIndex`: appending, trimming from the front, rescanning in chunks, match navigation, and
//! the index over a real `LogService` session fed by a `FakeLogPort`.

use std::sync::Arc;

use super::{Harness, burst, line};
use crate::logs::{LogBuffer, LogConfig, LogEntry, LogFilter, MatchIndex};

fn buffer_of(capacity: usize, texts: &[&str]) -> LogBuffer {
    let mut buffer = LogBuffer::new(capacity);
    buffer.extend(texts.iter().enumerate().map(|(i, text)| {
        let mut entry = LogEntry::new(line(i));
        entry.text = Arc::from(*text);
        entry
    }));
    buffer
}

fn index(pattern: &str) -> MatchIndex {
    MatchIndex::new(Arc::new(LogFilter::new(pattern).compile().unwrap()))
}

fn seqs(index: &MatchIndex) -> Vec<u64> {
    index.iter().collect()
}

#[test]
fn appended_lines_are_tested_once_as_they_arrive() {
    let mut buffer = buffer_of(100, &["a error", "b", "c error"]);
    let mut index = index("error");
    let change = index.catch_up(&buffer);
    assert_eq!(
        (change.appended, change.tested, change.dropped_front),
        (2, 3, 0)
    );
    assert_eq!(seqs(&index), [0, 2]);

    buffer.extend([LogEntry::new(line(3)), {
        let mut e = LogEntry::new(line(4));
        e.text = Arc::from("d error");
        e
    }]);
    let change = index.catch_up(&buffer);
    assert_eq!(
        (change.appended, change.tested),
        (1, 2),
        "only the new lines"
    );
    assert_eq!(seqs(&index), [0, 2, 4]);
    assert!(index.is_caught_up(&buffer));
    assert_eq!(index.catch_up(&buffer).tested, 0);
}

#[test]
fn matches_of_dropped_lines_leave_from_the_front() {
    let mut buffer = buffer_of(4, &["e0 error", "x", "e2 error", "x"]);
    let mut index = index("error");
    index.catch_up(&buffer);
    assert_eq!(seqs(&index), [0, 2]);

    // Three more lines push seqs 0..3 out of the 4-line ring.
    buffer.extend(["x", "e5 error", "x"].iter().enumerate().map(|(i, text)| {
        let mut e = LogEntry::new(line(i));
        e.text = Arc::from(*text);
        e
    }));
    assert_eq!(buffer.first_seq(), 3);
    let change = index.catch_up(&buffer);
    assert_eq!((change.dropped_front, change.appended), (2, 1));
    assert_eq!(seqs(&index), [5]);
}

#[test]
fn lines_dropped_before_the_index_looked_are_never_tested() {
    let mut buffer = buffer_of(2, &["error"]);
    let mut index = index("error");
    index.catch_up(&buffer);
    buffer.extend((0..10).map(|i| LogEntry::new(line(i))));
    let change = index.catch_up(&buffer);
    assert_eq!(change.tested, 2, "the ring holds two lines");
    assert_eq!(change.dropped_front, 1);
    assert!(index.is_empty());
}

#[test]
fn a_rescan_in_chunks_equals_one_pass() {
    let texts: Vec<String> = (0..1_000)
        .map(|i| format!("line {i} {}", if i % 7 == 0 { "error" } else { "ok" }))
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let buffer = buffer_of(2_000, &refs);
    let mut whole = index("error");
    whole.catch_up(&buffer);
    let mut chunked = index("error");
    let mut chunks = 0;
    while !chunked.is_caught_up(&buffer) {
        assert!(chunked.scan(&buffer, 64).tested <= 64);
        chunks += 1;
    }
    assert_eq!(chunks, 16);
    assert_eq!(seqs(&chunked), seqs(&whole));
    assert_eq!(whole.len(), 143);
}

#[test]
fn a_restarted_buffer_empties_the_index() {
    let buffer = buffer_of(10, &["error", "error", "error"]);
    let mut index = index("error");
    index.catch_up(&buffer);
    assert_eq!(index.len(), 3);
    let fresh = buffer_of(10, &["error"]);
    let change = index.catch_up(&fresh);
    assert_eq!((change.dropped_front, change.appended), (3, 1));
    assert_eq!(seqs(&index), [0]);
}

#[test]
fn lookups_by_seq() {
    let buffer = buffer_of(100, &["e", "x", "e", "x", "x", "e"]);
    let mut index = index("e$");
    index.catch_up(&buffer);
    assert_eq!(seqs(&index), [0, 2, 5]);
    assert_eq!(index.position(2), Some(1));
    assert_eq!(index.position(3), None);
    assert!(index.contains(5) && !index.contains(4));
    assert_eq!(index.rank(3), 2);
}

#[test]
fn next_and_previous_wrap_around() {
    let buffer = buffer_of(100, &["e", "x", "e", "x", "e"]);
    let mut index = index("^e$");
    index.catch_up(&buffer);
    assert_eq!(seqs(&index), [0, 2, 4]);
    // From nothing: next starts at the anchor (the top line), previous at the newest.
    assert_eq!(index.next(None, 1), Some(2));
    assert_eq!(
        index.next(None, 99),
        Some(0),
        "nothing after the anchor: wrap"
    );
    assert_eq!(index.prev(None), Some(4));
    // Walking forward wraps from the last to the first.
    assert_eq!(index.next(Some(0), 0), Some(2));
    assert_eq!(index.next(Some(2), 0), Some(4));
    assert_eq!(index.next(Some(4), 0), Some(0));
    // And backward from the first to the last.
    assert_eq!(index.prev(Some(4)), Some(2));
    assert_eq!(index.prev(Some(0)), Some(4));
    // No match: nowhere to go.
    let none = self::index("zzz");
    assert_eq!(none.next(None, 0), None);
    assert_eq!(none.prev(Some(3)), None);
}

#[test]
fn a_current_match_that_was_dropped_is_skipped_by_seq() {
    let mut buffer = buffer_of(4, &["e", "x", "e", "x"]);
    let mut index = index("^e$");
    index.catch_up(&buffer);
    // The user is on seq 0; the ring then drops seqs 0..3.
    buffer.extend((0..3).map(|i| {
        let mut e = LogEntry::new(line(i));
        e.text = Arc::from(if i == 1 { "e" } else { "x" });
        e
    }));
    index.catch_up(&buffer);
    assert_eq!(seqs(&index), [5]);
    assert_eq!(
        index.next(Some(0), 0),
        Some(5),
        "0 is gone: the next retained match"
    );
    assert_eq!(
        index.prev(Some(0)),
        Some(5),
        "nothing before it: wrap to the newest"
    );
    assert_eq!(index.position(0), None, "so its ordinal is gone too");
}

#[test]
fn an_index_over_a_streaming_session_counts_and_tracks_the_ring() {
    let mut harness = Harness::with_config(LogConfig {
        buffer_lines: 50,
        ..LogConfig::default()
    });
    let session = harness.follow(burst(120));
    let mut index = index("line 1");
    session.read(|buffer, _| index.catch_up(buffer));
    // Retained: seqs 70..120. Matches of "line 1": 100..=119 (20 lines).
    assert_eq!(index.len(), 20);
    assert_eq!(index.first(), Some(100));
    assert_eq!(index.last(), Some(119));
    assert_eq!(index.next(Some(119), 0), Some(100));
    assert_eq!(index.prev(Some(100)), Some(119));
}
