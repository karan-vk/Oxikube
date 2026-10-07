//! Cancel on drop, a read that does not follow, and the bounded merged buffer.

use std::time::Duration;

use oxikube_ports::LogOptions;
use oxikube_testkit::Timeline;

use super::{Harness, line, texts, web, web_pod, web_ref};
use crate::logs::{AggregateSpec, EndReason, LogConfig, LogState};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn dropping_the_session_aborts_the_watch_and_every_stream() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::new().keep_open());
    h.script(Timeline::new().keep_open());
    let session = h.open_web();
    assert_eq!(h.logs.live_streams(), 2);
    assert_eq!(h.resources.live_watches(), 1);
    let reader = session.reader();

    drop(session);
    h.settle();
    assert_eq!(h.logs.live_streams(), 0, "a stream outlived its session");
    assert_eq!(
        h.resources.live_watches(),
        0,
        "the pod watch outlived its session"
    );
    assert_eq!(reader.state(), LogState::Ended(EndReason::Cancelled));
}

#[test]
fn a_session_dropped_before_its_task_ran_opens_nothing() {
    let h = Harness::new();
    h.seed([web(), web_pod("web-a")]);
    let session = h.service.open_aggregate(
        h.ports(),
        AggregateSpec::of(&web_ref()).unwrap(),
        LogOptions::follow(),
    );
    drop(session);
    let mut h = h;
    h.settle();
    assert!(h.logs.recorded_calls().is_empty());
    assert!(h.resources.recorded_calls().is_empty());
}

#[test]
fn a_read_that_does_not_follow_lists_the_pods_once_and_completes_when_every_stream_does() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::immediate([
        line("web-a", 20, "a20"),
        line("web-a", 40, "a40"),
    ]));
    h.script(Timeline::immediate([
        line("web-b", 10, "b10"),
        line("web-b", 30, "b30"),
    ]));
    let options = LogOptions {
        follow: false,
        tail_lines: Some(100),
        ..LogOptions::default()
    };
    let session = h.open(AggregateSpec::of(&web_ref()).unwrap(), options);
    h.run_for(ms(200));
    assert_eq!(texts(&session), ["b10", "a20", "b30", "a40"]);
    assert_eq!(session.state(), LogState::Ended(EndReason::Completed));
    assert_eq!(
        h.resources.live_watches(),
        0,
        "no watch for a read that does not follow"
    );
    assert!(
        h.logs.recorded_calls().iter().all(|call| matches!(
            call,
            oxikube_testkit::LogCall::StreamLogs { options, .. }
                if !options.follow && options.tail_lines == Some(100)
        )),
        "the read's options reach every stream"
    );
}

#[test]
fn the_merged_buffer_is_one_ring_of_buffer_lines_with_a_truncated_marker() {
    let mut h = Harness::with_config(LogConfig {
        buffer_lines: 100,
        ..LogConfig::default()
    });
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    let burst = |pod: &str| {
        Timeline::immediate((0..200).map(|i| line(pod, i, &format!("{pod}-{i}")))).keep_open()
    };
    h.script(burst("web-a"));
    h.script(burst("web-b"));
    let session = h.open_web();
    h.run_for(ms(1_500));
    session.read(|buffer, _| {
        assert_eq!(buffer.len(), 100, "100 lines in all, not 100 per pod");
        assert!(buffer.is_truncated());
        assert_eq!(buffer.dropped(), 300);
    });
    // The newest lines are the ones kept, still merged by timestamp.
    let kept = texts(&session);
    let newest: Vec<String> = (150..200)
        .flat_map(|i| [format!("web-a-{i}"), format!("web-b-{i}")])
        .collect();
    assert_eq!(kept, newest, "the newest 100 lines, both pods, in order");
    let last_two: Vec<&str> = kept.iter().rev().take(2).map(String::as_str).collect();
    assert_eq!(last_two, ["web-b-199", "web-a-199"]);
}

#[test]
fn a_changed_buffer_bound_reaches_the_merged_buffer() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a")]);
    h.script(Timeline::immediate((0..50).map(|i| line("web-a", i, "x"))).keep_open());
    let session = h.open_web();
    h.run_for(ms(1_000));
    assert_eq!(session.len(), 50);
    h.service.set_buffer_lines(100);
    // 100 is the floor; a smaller value would be clamped, which the single-session tests cover.
    session.read(|buffer, _| assert_eq!(buffer.capacity(), 100));
}

/// While a stream is slow to open the merge holds every line back, but only as many as the buffer
/// could keep: a chatty pod cannot grow the waiting lines for the length of the start-up barrier.
#[test]
fn lines_held_by_the_start_up_barrier_stay_bounded_by_the_buffer() {
    use std::sync::Arc;

    let mut h = Harness::with_config(LogConfig {
        buffer_lines: 100,
        ..LogConfig::default()
    });
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::immediate((0..500).map(|i| line("web-a", i, &format!("a{i}")))).keep_open());
    // web-b's stream never opens, so the barrier stays closed for its 2 s.
    let ports = super::super::AggregatePorts {
        logs: Arc::new(NeverOpens {
            inner: h.logs.clone(),
            pod: "web-b",
        }),
        resources: h.resources.clone(),
    };
    let session = h.open_with(
        ports,
        AggregateSpec::of(&web_ref()).unwrap(),
        LogOptions::follow(),
    );
    h.run_for(ms(1_000));
    session.read(|buffer, _| {
        assert_eq!(
            buffer.len() + buffer.dropped() as usize,
            400,
            "all but the 100 still waiting"
        );
        assert_eq!(buffer.len(), 100);
    });
    let kept = texts(&session);
    assert_eq!(kept.first().map(String::as_str), Some("a300"));
    assert_eq!(
        kept.last().map(String::as_str),
        Some("a399"),
        "oldest first: the newest 100 wait"
    );
}

/// A `LogPort` whose stream for `pod` never opens.
struct NeverOpens {
    inner: std::sync::Arc<oxikube_testkit::FakeLogPort>,
    pod: &'static str,
}

#[async_trait::async_trait]
impl oxikube_ports::LogPort for NeverOpens {
    async fn stream_logs(
        &self,
        namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> oxikube_domain::OxiResult<oxikube_ports::LogStream> {
        if pod == self.pod {
            futures::future::pending::<()>().await;
        }
        self.inner.stream_logs(namespace, pod, options).await
    }
}
