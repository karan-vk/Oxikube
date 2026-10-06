//! Batched commits: a burst is a few batches, a trickle is one batch per tick, order is kept.

use std::time::Duration;

use super::{Harness, burst, line};
use crate::logs::{LogConfig, LogState};
use oxikube_testkit::Timeline;

fn roomy() -> LogConfig {
    LogConfig {
        buffer_lines: 20_000,
        ..LogConfig::default()
    }
}

#[test]
fn a_burst_of_ten_thousand_lines_is_a_few_batches_in_order_without_loss() {
    let mut h = Harness::with_config(roomy());
    let session = h.follow_flushed(burst(10_000).keep_open());
    assert_eq!(session.len(), 10_000);
    // max_batch is 2 048 lines: five batches, not ten thousand commits.
    assert!(session.batches() <= 5, "{} batches", session.batches());
    session.read(|buffer, state| {
        assert_eq!(*state, LogState::Streaming);
        for (i, entry) in buffer.iter().enumerate() {
            assert_eq!(entry.seq, i as u64);
            assert_eq!(entry.text.as_ref(), format!("line {i}"));
        }
    });
}

#[test]
fn lines_become_visible_on_the_flush_tick_not_one_by_one() {
    let mut h = Harness::with_config(roomy());
    // 100 lines, one per millisecond: 32 ms ticks gather about 32 each.
    let mut timeline = Timeline::new();
    for i in 0..100 {
        timeline = timeline.ok_at(Duration::from_millis(i as u64), line(i));
    }
    let session = h.follow(timeline.keep_open());
    // The first line opened a batch; the tick has not fired: nothing is committed yet.
    assert_eq!(session.len(), 0);
    assert_eq!(session.batches(), 0);

    for _ in 0..200 {
        h.advance(Duration::from_millis(1));
    }
    assert_eq!(session.len(), 100);
    assert!(
        (2..=6).contains(&session.batches()),
        "{} batches for 100 ms of stream",
        session.batches()
    );
}

#[test]
fn a_quiet_stream_commits_when_it_ends_without_waiting_for_the_tick() {
    let mut h = Harness::new();
    let session = h.open(
        burst(3),
        crate::logs::LogTarget::pod("default", "web-0"),
        oxikube_ports::LogOptions::default(),
    );
    assert_eq!(session.len(), 3);
    assert_eq!(session.batches(), 1);
}

#[test]
fn a_chatty_pod_at_five_thousand_lines_per_second_stays_bounded_and_batched() {
    let mut h = Harness::with_config(LogConfig {
        buffer_lines: 1_000,
        ..LogConfig::default()
    });
    // 20 000 lines over 4 seconds: 5 lines per millisecond.
    let mut timeline = Timeline::new();
    for i in 0..20_000 {
        timeline = timeline.ok_at(Duration::from_micros(200 * i as u64), line(i));
    }
    let session = h.follow(timeline.keep_open());
    for _ in 0..4_100 {
        h.advance(Duration::from_millis(1));
    }
    assert_eq!(session.len(), 1_000);
    session.read(|buffer, _| {
        assert_eq!(buffer.next_seq(), 20_000);
        assert_eq!(buffer.dropped(), 19_000);
        assert_eq!(buffer.last().unwrap().text.as_ref(), "line 19999");
    });
    // About one batch per 32 ms tick over 4 s, nowhere near one per line.
    assert!(session.batches() < 200, "{} batches", session.batches());
}

#[test]
fn lines_keep_the_servers_timestamps_and_origin() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(3).keep_open());
    session.read(|buffer, _| {
        let first = buffer.get(0).unwrap();
        assert_eq!(first.ts, super::ts(0));
        assert_eq!(buffer.get(2).unwrap().ts, super::ts(2));
        assert_eq!(first.pod.as_ref(), "web-0");
        assert_eq!(first.container.as_ref(), "app");
    });
}
