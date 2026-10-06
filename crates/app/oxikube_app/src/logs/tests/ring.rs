//! The ring buffer: bounds, the truncated marker, and index / seq / range access.

use super::line;
use crate::logs::{LogBuffer, LogEntry};

fn entries(range: std::ops::Range<usize>) -> Vec<LogEntry> {
    range.map(|i| LogEntry::new(line(i))).collect()
}

fn texts(buffer: &LogBuffer) -> Vec<String> {
    buffer.iter().map(|e| e.text.to_string()).collect()
}

#[test]
fn a_new_buffer_is_empty_and_not_truncated() {
    let buffer = LogBuffer::new(10);
    assert!(buffer.is_empty());
    assert_eq!((buffer.len(), buffer.capacity()), (0, 10));
    assert_eq!((buffer.first_seq(), buffer.next_seq()), (0, 0));
    assert!(!buffer.is_truncated());
    assert_eq!(buffer.dropped(), 0);
}

#[test]
fn lines_are_numbered_in_order() {
    let mut buffer = LogBuffer::new(10);
    assert_eq!(buffer.extend(entries(0..3)), 0);
    assert_eq!(buffer.extend(entries(3..5)), 0);
    let seqs: Vec<u64> = buffer.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [0, 1, 2, 3, 4]);
    assert_eq!(buffer.next_seq(), 5);
    assert_eq!(buffer.last().unwrap().text.as_ref(), "line 4");
}

#[test]
fn pushing_twice_the_capacity_keeps_the_newest_and_counts_the_rest() {
    let mut buffer = LogBuffer::new(100);
    // One batch of 2x capacity: the front of the batch itself does not fit.
    assert_eq!(buffer.extend(entries(0..200)), 100);
    assert_eq!(buffer.len(), 100);
    assert!(buffer.is_truncated());
    assert_eq!(buffer.dropped(), 100);
    assert_eq!((buffer.first_seq(), buffer.next_seq()), (100, 200));
    assert_eq!(buffer.get(0).unwrap().text.as_ref(), "line 100");
    assert_eq!(buffer.get(99).unwrap().text.as_ref(), "line 199");
    assert!(buffer.get(100).is_none());
}

#[test]
fn index_and_seq_map_onto_each_other_after_drops() {
    let mut buffer = LogBuffer::new(5);
    buffer.extend(entries(0..12));
    assert_eq!(buffer.first_seq(), 7);
    for seq in 7..12 {
        let index = buffer.index_of(seq).unwrap();
        assert_eq!(index as u64, seq - 7);
        let by_seq = buffer.get_seq(seq).unwrap();
        assert_eq!(by_seq.seq, seq);
        assert_eq!(by_seq, buffer.get(index).unwrap());
    }
    // Dropped and not yet written seqs are not there.
    assert!(buffer.index_of(6).is_none());
    assert!(buffer.index_of(12).is_none());
    assert!(buffer.get_seq(0).is_none());
}

#[test]
fn ranges_are_clamped_to_what_is_retained() {
    let mut buffer = LogBuffer::new(5);
    buffer.extend(entries(0..12));
    let by_index: Vec<u64> = buffer.range(1..3).map(|e| e.seq).collect();
    assert_eq!(by_index, [8, 9]);
    assert_eq!(buffer.range(3..100).count(), 2);
    assert_eq!(buffer.range(50..60).count(), 0);
    let (start, end) = (4, 2);
    assert_eq!(
        buffer.range(start..end).count(),
        0,
        "a reversed range is empty"
    );

    let by_seq: Vec<u64> = buffer.range_seq(9..11).map(|e| e.seq).collect();
    assert_eq!(by_seq, [9, 10]);
    // Starts before the oldest retained line, ends past the newest.
    assert_eq!(buffer.range_seq(0..100).count(), 5);
    assert_eq!(
        buffer.range_seq(0..8).map(|e| e.seq).collect::<Vec<_>>(),
        [7]
    );
    assert_eq!(buffer.range_seq(20..30).count(), 0);
}

#[test]
fn shrinking_drops_the_oldest_and_growing_keeps_everything() {
    let mut buffer = LogBuffer::new(10);
    buffer.extend(entries(0..10));
    assert_eq!(buffer.set_capacity(4), 6);
    assert_eq!(texts(&buffer), ["line 6", "line 7", "line 8", "line 9"]);
    assert_eq!((buffer.first_seq(), buffer.dropped()), (6, 6));
    assert_eq!(buffer.set_capacity(20), 0);
    buffer.extend(entries(10..14));
    assert_eq!(buffer.len(), 8);
    assert_eq!(buffer.first_seq(), 6);
}

#[test]
fn the_capacity_is_at_least_one_line() {
    let mut buffer = LogBuffer::new(0);
    buffer.extend(entries(0..3));
    assert_eq!(buffer.len(), 1);
    assert_eq!(buffer.last().unwrap().text.as_ref(), "line 2");
}

#[test]
fn memory_stays_bounded_over_a_long_stream() {
    let mut buffer = LogBuffer::new(1_000);
    for batch in 0..200 {
        let start = batch * 500;
        buffer.extend(entries(start..start + 500));
    }
    assert_eq!(buffer.len(), 1_000);
    assert_eq!(buffer.dropped(), 100_000 - 1_000);
    assert_eq!(buffer.next_seq(), 100_000);
}

#[test]
fn an_entry_keeps_the_servers_timestamp_and_the_lines_origin() {
    let entry = LogEntry::new(line(7));
    assert_eq!(entry.ts, super::ts(7));
    assert_eq!(entry.pod.as_ref(), "web-0");
    assert_eq!(entry.container.as_ref(), "app");
    assert!(!entry.truncated);
}
