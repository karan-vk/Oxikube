//! The user's "clear": the buffer empties, the stream goes on, seqs are never reused and the
//! cleared lines are not "dropped".

use std::task::Poll;
use std::time::Duration;

use futures::Stream;
use futures::task::noop_waker;
use oxikube_testkit::Timeline;

use super::{Harness, burst, line};
use crate::logs::{LogBuffer, LogDelta, LogDeltas, LogEntry};

fn poll(deltas: &mut LogDeltas) -> Option<LogDelta> {
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    match std::pin::Pin::new(deltas).poll_next(&mut cx) {
        Poll::Ready(item) => item,
        Poll::Pending => None,
    }
}

#[test]
fn clearing_a_buffer_keeps_the_seqs_and_is_not_a_drop() {
    let mut buffer = LogBuffer::new(10);
    buffer.extend((0..4).map(|i| LogEntry::new(line(i))));
    assert_eq!(buffer.clear(), 4);
    assert!(buffer.is_empty());
    assert_eq!((buffer.first_seq(), buffer.next_seq()), (4, 4));
    assert_eq!((buffer.cleared(), buffer.dropped()), (4, 0));
    assert!(
        !buffer.is_truncated(),
        "cleared lines leave no truncated marker"
    );
    buffer.extend((4..6).map(|i| LogEntry::new(line(i))));
    let seqs: Vec<u64> = buffer.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [4, 5], "new lines carry on numbering");
    assert_eq!(buffer.clear(), 2);
    assert_eq!(buffer.clear(), 0);
    assert_eq!(buffer.cleared(), 6);
}

#[test]
fn only_lines_dropped_since_the_clear_count_as_missing() {
    let mut buffer = LogBuffer::new(10);
    buffer.extend((0..25).map(|i| LogEntry::new(line(i))));
    assert_eq!((buffer.dropped(), buffer.dropped_since_clear()), (15, 15));
    buffer.clear();
    assert_eq!(
        (buffer.dropped(), buffer.dropped_since_clear()),
        (15, 0),
        "the lifetime count stays, what is missing before the first line does not"
    );
    buffer.extend((25..32).map(|i| LogEntry::new(line(i))));
    assert_eq!(buffer.dropped_since_clear(), 0, "it still fits");
    buffer.extend((32..40).map(|i| LogEntry::new(line(i))));
    assert_eq!((buffer.dropped(), buffer.dropped_since_clear()), (20, 5));
}

#[test]
fn a_session_clears_while_streaming_and_the_delta_says_so() {
    let mut h = Harness::new();
    let session = h.follow(
        Timeline::new()
            .ok_at(Duration::ZERO, line(0))
            .ok_at(Duration::ZERO, line(1))
            .ok_at(Duration::from_secs(1), line(2))
            .keep_open(),
    );
    let mut deltas = session.deltas();
    h.advance(Duration::from_millis(100));
    let first = poll(&mut deltas).expect("the first lines");
    assert_eq!(first.appended, 0..2);

    assert_eq!(session.clear(), 2, "the next line gets seq 2");
    assert!(session.is_empty());
    let cleared = poll(&mut deltas).expect("a clear wakes the readers");
    assert_eq!((cleared.dropped_front, cleared.first_seq), (2, 2));
    assert!(cleared.appended.is_empty());

    // The stream was not touched: its next line arrives as seq 2.
    h.advance(Duration::from_secs(2));
    h.advance(Duration::from_millis(100));
    let next = poll(&mut deltas).expect("the stream goes on");
    assert_eq!((next.appended, next.first_seq), (2..3, 2));
    let texts = session.read(|buffer, _| {
        buffer
            .iter()
            .map(|e| (e.seq, e.text.to_string()))
            .collect::<Vec<_>>()
    });
    assert_eq!(texts, [(2, "line 2".to_owned())]);
}

#[test]
fn clearing_an_empty_session_is_harmless() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(0).keep_open());
    assert_eq!(session.clear(), 0);
    assert!(session.is_empty());
}
