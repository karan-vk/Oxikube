//! A stream that breaks while its pod runs: backoff, overlap dedupe, the retry cap, a container
//! that is still starting, and a reconnect by hand.

use std::time::Duration;

use oxikube_domain::log::LogLine;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{LogOptions, LogSince};
use oxikube_testkit::Timeline;

use super::{Harness, line, lines, texts, ts};
use crate::logs::{LogSession, LogState, ReconnectPolicy};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Lines `range` at once, then a dropped connection at 100 ms.
fn breaking(range: std::ops::Range<i64>) -> Timeline<LogLine> {
    range
        .map(line)
        .fold(Timeline::new(), |t, l| t.ok_at(Duration::ZERO, l))
        .err_at(ms(100), OxiError::network("connection reset by peer"))
}

fn numbered(range: std::ops::Range<i64>) -> Vec<String> {
    range.map(|i| format!("line {i}")).collect()
}

#[test]
fn a_broken_stream_reconnects_from_the_overlap_without_duplicate_lines() {
    let mut h = Harness::new();
    h.script(breaking(0..5));
    // The reopened stream replays from 2 s before the last line (line 4): lines 2..5 again.
    h.script(lines(2..8).keep_open());
    let session = h.open_plain(LogOptions::follow().tail_lines(100));
    h.run_for(ms(150));
    let LogState::Reconnecting {
        attempt,
        max,
        failure,
    } = session.state()
    else {
        panic!("{:?}", session.state());
    };
    assert_eq!(
        (attempt, max),
        (1, 5),
        "Reconnecting 1/5 (logs.reconnect_retries)"
    );
    assert_eq!(failure.kind, ErrorKind::Network);
    assert_eq!(
        texts(&session),
        numbered(0..5),
        "the lines before the break stay"
    );
    assert_eq!(
        h.opens().len(),
        1,
        "nothing reopens before the pause is over"
    );

    h.run_for(ms(700));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), numbered(0..8), "no line twice, none lost");
    let reopen = &h.opens()[1];
    assert_eq!(reopen.since, Some(LogSince::Time(ts(2_000))));
    assert_eq!(reopen.tail_lines, None, "the overlap replaces the tail");
    assert!(reopen.timestamps && reopen.follow);
}

#[test]
fn retries_back_off_and_stop_at_the_cap_with_a_failure() {
    let mut h = Harness::new();
    h.service.set_reconnect_retries(2);
    h.script(breaking(0..1));
    for _ in 0..3 {
        h.logs
            .script()
            .stream_logs
            .push_err(OxiError::network("connection refused"));
    }
    let session = h.open_plain(LogOptions::follow());
    h.run_for(ms(150));
    assert!(matches!(
        session.state(),
        LogState::Reconnecting {
            attempt: 1,
            max: 2,
            ..
        }
    ));
    // The first pause is 500 ms (plus up to a quarter of jitter), the second about twice that.
    h.run_for(ms(700));
    assert_eq!(h.opens().len(), 2);
    assert!(matches!(
        session.state(),
        LogState::Reconnecting {
            attempt: 2,
            max: 2,
            ..
        }
    ));
    h.run_for(ms(800));
    assert_eq!(
        h.opens().len(),
        2,
        "the second pause is longer than the first"
    );
    h.run_for(ms(600));
    let LogState::Failed(failure) = session.state() else {
        panic!("{:?}", session.state());
    };
    assert!(failure.retryable && failure.kind == ErrorKind::Network);
    assert_eq!(h.opens().len(), 3, "the first open and two retries");
    h.run_for(Duration::from_secs(60));
    assert_eq!(h.opens().len(), 3, "no tight loop after giving up");
    assert_eq!(texts(&session), ["line 0"]);
}

#[test]
fn a_stream_that_delivers_lines_starts_the_count_again() {
    let mut h = Harness::new();
    h.service.set_reconnect_retries(1);
    h.script(breaking(0..2));
    h.script(breaking(1..4));
    h.script(breaking(3..6));
    h.script(lines(5..7).keep_open());
    let session = h.open_plain(LogOptions::follow());
    h.run_for(Duration::from_secs(5));
    assert_eq!(
        session.state(),
        LogState::Streaming,
        "three breaks, never two in a row"
    );
    assert_eq!(texts(&session), numbered(0..7));
}

#[test]
fn a_container_waiting_to_start_is_retried_until_it_streams() {
    let mut h = Harness::new();
    for _ in 0..2 {
        h.logs.script().stream_logs.push_err(OxiError::validation(
            "container \"app\" in pod \"web-0\" is waiting to start: ContainerCreating",
        ));
    }
    h.script(lines(0..2).keep_open());
    let session = h.open_plain(LogOptions::follow());
    assert_eq!(session.state(), LogState::Connecting, "not a failure");
    h.run_for(Duration::from_millis(2_100));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), numbered(0..2));
    assert_eq!(h.opens().len(), 3);
}

#[test]
fn a_denied_read_fails_at_once_and_is_not_retried() {
    let mut h = Harness::new();
    h.logs.script().stream_logs.push_err(OxiError::forbidden(
        "pods \"web-0\" is forbidden: cannot get pods/log",
    ));
    let session = h.open_plain(LogOptions::follow());
    h.run_for(Duration::from_secs(5));
    let LogState::Failed(failure) = session.state() else {
        panic!("{:?}", session.state());
    };
    assert_eq!(failure.kind, ErrorKind::Forbidden);
    assert_eq!(h.opens().len(), 1);
}

#[test]
fn the_retry_setting_applies_to_open_sessions() {
    let mut h = Harness::new();
    h.script(breaking(0..1));
    let session = h.open_plain(LogOptions::follow());
    h.service.set_reconnect_retries(0);
    h.run_for(ms(150));
    assert!(
        matches!(session.state(), LogState::Failed(_)),
        "0 retries: the break is final, {:?}",
        session.state()
    );
}

#[test]
fn a_read_that_does_not_follow_is_never_reconnected() {
    let mut h = Harness::new();
    h.script(breaking(0..2));
    let session = h.open_plain(LogOptions::default().tail_lines(10));
    h.run_for(Duration::from_secs(2));
    assert!(matches!(session.state(), LogState::Failed(_)));
    assert_eq!(h.opens().len(), 1);
}

#[test]
fn a_session_reconnected_by_hand_continues_after_the_lines_it_kept() {
    let mut h = Harness::with_config(crate::logs::LogConfig {
        reconnect: ReconnectPolicy::default(),
        ..crate::logs::LogConfig::default()
    });
    h.service.set_reconnect_retries(0);
    h.script(breaking(0..4));
    let mut session = h.open_plain(LogOptions::follow());
    h.run_for(ms(150));
    assert!(matches!(session.state(), LogState::Failed(_)));
    assert_eq!(texts(&session), numbered(0..4));

    h.script(lines(1..6).keep_open());
    let mut deltas = session.deltas();
    assert!(session.reconnect(), "a failed session can be reconnected");
    assert!(!session.reconnect(), "not while it reads");
    h.run_for(ms(100));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(texts(&session), numbered(0..6), "kept, and no line twice");
    assert_eq!(h.opens()[1].since, Some(LogSince::Time(ts(1_000))));
    // A new reader sees the whole buffer and the new state.
    use futures::StreamExt as _;
    let delta = futures::executor::block_on(deltas.next()).expect("a delta");
    assert_eq!(delta.state, LogState::Streaming);

    // A dropped session cannot be reconnected; a cancelled one neither.
    let reader = session.reader();
    drop(session);
    assert!(reader.state().is_terminal());
}

/// A line `text` of `web-0` stamped `at_ms` milliseconds in.
fn stamped(at_ms: i64, text: &str) -> LogLine {
    LogLine::new(ts(at_ms), "web-0", "app", text)
}

/// The server timestamps of the buffer, in milliseconds into the fixture's hour, oldest first.
fn stamps(session: &LogSession) -> Vec<i64> {
    session.read(|buffer, _| {
        buffer
            .iter()
            .map(|e| e.ts.as_millisecond() - ts(0).as_millisecond())
            .collect()
    })
}

/// The dedupe rule end to end: a line the reopened stream delivers again (same server timestamp,
/// same text) appears once, a line an application repeats (same text, other timestamp) is kept,
/// and two identical lines the server really sent twice are both kept the first time round.
#[test]
fn identical_text_at_other_timestamps_is_kept_and_only_a_true_replay_is_dropped() {
    let mut h = Harness::new();
    h.script(
        Timeline::new()
            .ok_at(Duration::ZERO, stamped(1_000, "tick"))
            .ok_at(Duration::ZERO, stamped(2_000, "tick"))
            .ok_at(Duration::ZERO, stamped(3_000, "tick"))
            // The same line twice from the server: a real repeat, not a replay.
            .ok_at(Duration::ZERO, stamped(3_000, "tick"))
            .ok_at(Duration::ZERO, stamped(4_000, "tick"))
            .err_at(ms(100), OxiError::network("connection reset by peer")),
    );
    // Reopened `since` 2 s before the last line: the server replays 2000..=4000 again, with a
    // line the first stream never had inside that window, then goes on.
    h.script(
        Timeline::new()
            .ok_at(Duration::ZERO, stamped(2_000, "tick"))
            .ok_at(Duration::ZERO, stamped(2_500, "tick"))
            .ok_at(Duration::ZERO, stamped(3_000, "tick"))
            .ok_at(Duration::ZERO, stamped(3_000, "tick"))
            .ok_at(Duration::ZERO, stamped(4_000, "tick"))
            .ok_at(Duration::ZERO, stamped(4_500, "tick"))
            .ok_at(Duration::ZERO, stamped(5_000, "tick"))
            .keep_open(),
    );
    let session = h.open_plain(LogOptions::follow().tail_lines(100));
    h.run_for(ms(150));
    assert_eq!(stamps(&session), [1_000, 2_000, 3_000, 3_000, 4_000]);

    h.run_for(ms(700));
    assert_eq!(session.state(), LogState::Streaming);
    assert_eq!(
        stamps(&session),
        [1_000, 2_000, 3_000, 3_000, 4_000, 2_500, 4_500, 5_000],
        "the replayed 2000, 3000, 3000 and 4000 are gone once; the unseen 2500 and the new \
         4500 and 5000 are kept although every line says `tick`"
    );
    assert!(texts(&session).iter().all(|t| t == "tick"));
}
