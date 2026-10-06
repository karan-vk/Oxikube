//! Delta cursors: the window a consumer rebuilds from deltas always equals the buffer.

use std::task::Poll;
use std::time::Duration;

use futures::Stream;
use futures::task::noop_waker;
use oxikube_ports::LogOptions;

use super::{Harness, burst, line};
use crate::logs::{EndReason, LogConfig, LogDeltas, LogState, LogTarget};
use oxikube_testkit::Timeline;

/// Polls once without blocking: `Some(None)` is the end, `None` is "nothing new".
fn poll(deltas: &mut LogDeltas) -> Option<Option<crate::logs::LogDelta>> {
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    match std::pin::Pin::new(deltas).poll_next(&mut cx) {
        Poll::Ready(item) => Some(item),
        Poll::Pending => None,
    }
}

fn small(buffer_lines: usize) -> LogConfig {
    LogConfig {
        buffer_lines: crate::logs::MIN_BUFFER_LINES.max(buffer_lines),
        ..LogConfig::default()
    }
}

#[test]
fn the_first_delta_carries_everything_retained_and_the_state() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(5).keep_open());
    let mut deltas = session.deltas();
    let first = poll(&mut deltas).unwrap().unwrap();
    assert_eq!(first.appended, 0..5);
    assert_eq!((first.dropped_front, first.first_seq), (0, 0));
    assert_eq!(first.state, LogState::Streaming);
    // Nothing happened since: pending, not a repeat.
    assert!(poll(&mut deltas).is_none());
}

#[test]
fn a_later_delta_has_only_what_is_new() {
    let mut h = Harness::new();
    let session = h.follow(
        Timeline::new()
            .ok_at(Duration::ZERO, line(0))
            .ok_at(Duration::from_secs(1), line(1))
            .ok_at(Duration::from_secs(1), line(2))
            .keep_open(),
    );
    let mut deltas = session.deltas();
    h.advance(Duration::from_millis(100));
    assert_eq!(poll(&mut deltas).unwrap().unwrap().appended, 0..1);
    h.advance(Duration::from_secs(2));
    h.advance(Duration::from_millis(40));
    let next = poll(&mut deltas).unwrap().unwrap();
    assert_eq!(next.appended, 1..3);
    assert_eq!(next.dropped_front, 0);
}

#[test]
fn a_burst_past_the_capacity_replaces_the_whole_window() {
    let mut h = Harness::with_config(small(100));
    let mut timeline = Timeline::new();
    for i in 0..330 {
        let at = if i < 30 {
            Duration::ZERO
        } else {
            Duration::from_secs(1)
        };
        timeline = timeline.ok_at(at, line(i));
    }
    let session = h.follow(timeline.keep_open());
    h.advance(Duration::from_millis(40));
    let mut deltas = session.deltas();
    let first = poll(&mut deltas).unwrap().unwrap();
    assert_eq!((first.appended, first.dropped_front), (0..30, 0));

    // 300 more lines: the 30 the consumer holds are gone, and so are 200 of the new ones.
    h.advance(Duration::from_secs(2));
    h.advance(Duration::from_millis(40));
    let delta = poll(&mut deltas).unwrap().unwrap();
    assert_eq!(delta.first_seq, 230);
    assert_eq!(delta.appended, 230..330);
    assert_eq!(delta.dropped_front, 30);
    // 30 - 30 + 100 lines: the window is exactly the buffer.
    assert_eq!(session.len(), 100);
}

#[test]
fn a_consumer_that_fell_behind_gets_one_delta_and_a_consistent_window() {
    let mut h = Harness::with_config(small(100));
    let mut timeline = Timeline::new();
    for i in 0..400 {
        let at = Duration::from_millis(40 * (i as u64 / 40));
        timeline = timeline.ok_at(at, line(i));
    }
    let session = h.follow(timeline.keep_open());
    let mut deltas = session.deltas();

    let (mut first, mut end) = (0u64, 0u64);
    let mut apply = |delta: crate::logs::LogDelta| {
        let len = end - first;
        let new_len =
            len - delta.dropped_front as u64 + (delta.appended.end - delta.appended.start);
        first = delta.first_seq;
        end = delta.appended.end;
        assert_eq!(end - first, new_len, "length invariant of {delta:?}");
    };
    // Poll only every 200 ms of stream time: several batches, some of them dropped, per delta.
    for _ in 0..12 {
        h.advance(Duration::from_millis(200));
        if let Some(Some(delta)) = poll(&mut deltas) {
            apply(delta);
        }
    }
    session.read(|buffer, _| {
        assert_eq!((first, end), (buffer.first_seq(), buffer.next_seq()));
    });
    assert_eq!(end, 400);
    assert_eq!(first, 300);
}

#[test]
fn every_reader_has_its_own_cursor() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(4).keep_open());
    let mut a = session.deltas();
    let mut b = session.reader().deltas();
    assert_eq!(poll(&mut a).unwrap().unwrap().appended, 0..4);
    assert_eq!(poll(&mut b).unwrap().unwrap().appended, 0..4);
}

#[test]
fn the_stream_ends_after_the_delta_that_carries_a_terminal_state() {
    let mut h = Harness::new();
    let session = h.open(
        burst(3),
        LogTarget::pod("default", "web-0"),
        LogOptions::default(),
    );
    let mut deltas = session.deltas();
    let delta = poll(&mut deltas).unwrap().unwrap();
    assert_eq!(delta.appended, 0..3);
    assert_eq!(delta.state, LogState::Ended(EndReason::Completed));
    assert_eq!(poll(&mut deltas), Some(None));
}

#[test]
fn dropping_the_session_ends_its_streams_with_a_cancelled_state() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(2).keep_open());
    let mut deltas = session.deltas();
    let _ = poll(&mut deltas);
    drop(session);
    let delta = poll(&mut deltas).unwrap().unwrap();
    assert_eq!(delta.state, LogState::Ended(EndReason::Cancelled));
    assert_eq!(poll(&mut deltas), Some(None));
}

#[test]
fn a_waiting_consumer_is_woken_by_the_next_batch() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Count(AtomicUsize);
    impl futures::task::ArcWake for Count {
        fn wake_by_ref(arc_self: &Arc<Self>) {
            arc_self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let waker = futures::task::waker(count.clone());
    let mut cx = std::task::Context::from_waker(&waker);

    let mut h = Harness::new();
    let session = h.follow(
        Timeline::new()
            .ok_at(Duration::from_secs(1), line(0))
            .keep_open(),
    );
    let mut deltas = session.deltas();
    // Streaming is news; take it, then wait.
    assert!(
        std::pin::Pin::new(&mut deltas)
            .poll_next(&mut cx)
            .is_ready()
    );
    assert!(
        std::pin::Pin::new(&mut deltas)
            .poll_next(&mut cx)
            .is_pending()
    );
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
    h.advance(Duration::from_secs(2));
    h.advance(Duration::from_millis(40));
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    assert!(
        std::pin::Pin::new(&mut deltas)
            .poll_next(&mut cx)
            .is_ready()
    );
}
