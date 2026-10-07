//! The merge: lines of several pods by server timestamp, with a stable tiebreak, whatever order
//! the streams delivered them in.

use std::sync::Arc;
use std::time::Duration;

use oxikube_ports::{ClockPort as _, LogOptions, LogPort, LogStream};
use oxikube_testkit::{FakeClockPort, FakeLogPort, ScriptedFeed, TICK, Timeline};

use super::{Harness, line, texts, web, web_pod};
use crate::logs::{AggregatePorts, AggregateSpec};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Three pods whose lines interleave by timestamp, delivered by streams with different
/// latencies: the merged buffer is in timestamp order, not in arrival order.
#[test]
fn lines_of_three_pods_merge_by_server_timestamp_not_arrival_order() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b"), web_pod("web-c")]);
    // Pods are opened in name order. a is quick; b's lines arrive 100 ms after a's; c's 160 ms.
    h.script(
        Timeline::new()
            .ok_at(ms(0), line("web-a", 10, "a10"))
            .ok_at(ms(0), line("web-a", 40, "a40"))
            .ok_at(ms(0), line("web-a", 70, "a70"))
            .keep_open(),
    );
    h.script(
        Timeline::new()
            .ok_at(ms(100), line("web-b", 20, "b20"))
            .ok_at(ms(100), line("web-b", 50, "b50"))
            .keep_open(),
    );
    h.script(
        Timeline::new()
            .ok_at(ms(160), line("web-c", 30, "c30"))
            .ok_at(ms(160), line("web-c", 60, "c60"))
            .keep_open(),
    );
    let session = h.open_web();
    h.run_for(ms(1_000));
    assert_eq!(
        texts(&session),
        ["a10", "b20", "c30", "a40", "b50", "c60", "a70"]
    );
}

#[test]
fn equal_timestamps_order_by_stream_then_by_position_in_the_stream() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    // b's lines arrive before a's, all with the same stamp.
    h.script(
        Timeline::new()
            .ok_at(ms(100), line("web-a", 5, "a-first"))
            .ok_at(ms(100), line("web-a", 5, "a-second"))
            .keep_open(),
    );
    h.script(
        Timeline::new()
            .ok_at(ms(0), line("web-b", 5, "b-first"))
            .ok_at(ms(0), line("web-b", 5, "b-second"))
            .keep_open(),
    );
    let session = h.open_web();
    h.run_for(ms(1_000));
    assert_eq!(
        texts(&session),
        ["a-first", "a-second", "b-first", "b-second"],
        "stream id (the pods' name order), then sequence in the stream"
    );
}

#[test]
fn a_pods_lines_never_change_order_even_when_its_clock_steps_back() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(
        Timeline::immediate([
            line("web-a", 100, "a1"),
            line("web-a", 200, "a2"),
            line("web-a", 150, "a3-clock-stepped-back"),
        ])
        .keep_open(),
    );
    h.script(Timeline::immediate([line("web-b", 160, "b1")]).keep_open());
    let session = h.open_web();
    h.run_for(ms(1_000));
    assert_eq!(
        texts(&session),
        ["a1", "b1", "a2", "a3-clock-stepped-back"],
        "a3 stays after a2: the order within one pod is the order the server sent"
    );
}

#[test]
fn the_merge_is_the_same_whichever_stream_answers_first() {
    let merged = |a_delay: u64, b_delay: u64| {
        let mut h = Harness::new();
        h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
        let burst = |pod: &str, delay: u64| {
            (0..6)
                .fold(Timeline::new(), |t, i| {
                    t.ok_at(ms(delay), line(pod, i / 2, &format!("{pod}{i}")))
                })
                .keep_open()
        };
        h.script(burst("web-a", a_delay));
        h.script(burst("web-b", b_delay));
        let session = h.open_web();
        h.run_for(ms(1_500));
        texts(&session)
    };
    let first = merged(0, 120);
    assert_eq!(first.len(), 12);
    assert_eq!(first, merged(120, 0), "same lines, same order");
    assert_eq!(first, merged(60, 60));
}

/// A `LogPort` that takes `delay` to open the stream of `pod`, like a slow connection.
struct SlowOpen {
    inner: Arc<FakeLogPort>,
    clock: Arc<FakeClockPort>,
    pod: &'static str,
    delay: Duration,
}

#[async_trait::async_trait]
impl LogPort for SlowOpen {
    async fn stream_logs(
        &self,
        namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> oxikube_domain::OxiResult<LogStream> {
        if pod == self.pod {
            self.clock.sleep(self.delay).await;
        }
        self.inner.stream_logs(namespace, pod, options).await
    }
}

#[test]
fn a_stream_that_is_slow_to_open_still_gets_its_older_lines_in_place() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::immediate([line("web-a", 500, "a-newer")]).keep_open());
    h.script(Timeline::immediate([line("web-b", 100, "b-older")]).keep_open());
    // b's stream opens 900 ms after a's: longer than the reorder window, so without the start-up
    // barrier a-newer would be committed first and b-older placed after it.
    let ports = AggregatePorts {
        logs: Arc::new(SlowOpen {
            inner: h.logs.clone(),
            clock: h.clock.clone(),
            pod: "web-b",
            delay: ms(900),
        }),
        resources: h.resources.clone(),
    };
    let spec = AggregateSpec::of(&super::web_ref()).unwrap();
    let session = h.open_with(ports, spec, LogOptions::follow());
    h.run_for(ms(800));
    assert!(
        texts(&session).is_empty(),
        "nothing is committed before every stream answered"
    );
    h.run_for(ms(1_500));
    assert_eq!(texts(&session), ["b-older", "a-newer"]);
}

#[test]
fn a_stream_that_never_opens_does_not_hold_the_others_back_for_ever() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::immediate([line("web-a", 500, "a-line")]).keep_open());
    h.script(Timeline::immediate([line("web-b", 100, "b-line")]).keep_open());
    let ports = AggregatePorts {
        logs: Arc::new(SlowOpen {
            inner: h.logs.clone(),
            clock: h.clock.clone(),
            pod: "web-b",
            delay: Duration::from_secs(3_600),
        }),
        resources: h.resources.clone(),
    };
    let spec = AggregateSpec::of(&super::web_ref()).unwrap();
    let session = h.open_with(ports, spec, LogOptions::follow());
    h.run_for(ms(1_800));
    assert!(texts(&session).is_empty());
    h.run_for(ms(1_000));
    assert_eq!(
        texts(&session),
        ["a-line"],
        "after startup_wait (2 s) the others go on"
    );
}

#[test]
fn a_backlog_older_than_the_window_goes_in_after_what_is_there() {
    let mut h = Harness::new();
    h.seed([web()]);
    ScriptedFeed::new()
        .initial([web_pod("web-a")])
        .add(1, web_pod("web-b"))
        .install(&h.resources);
    h.script(Timeline::immediate([line("web-a", 1_000, "a-live")]).keep_open());
    let session = h.open_web();
    h.run_for(ms(900));
    assert_eq!(texts(&session), ["a-live"]);

    // A pod that joins later (its pod appears at tick 1) brings lines stamped long before
    // everything committed: best effort, at the end, without holding anything back.
    h.script(Timeline::immediate([line("web-b", 10, "b-backlog")]).keep_open());
    h.run_for(TICK);
    h.run_for(ms(600));
    assert_eq!(texts(&session), ["a-live", "b-backlog"]);
}

/// The recorded fixture of three pods with skewed clocks, a clock that steps back and equal
/// timestamps (`fixtures/three-pods.log`): the merge is the one in `three-pods.merged`, whichever
/// way the streams' latencies are dealt out.
#[test]
fn a_recorded_log_of_three_pods_merges_to_its_expected_order_whatever_the_latencies() {
    struct Row {
        pod: String,
        arrival: u64,
        line: oxikube_domain::log::LogLine,
    }
    let rows: Vec<Row> = include_str!("fixtures/three-pods.log")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let [pod, arrival, ts, text] = l.splitn(4, '\t').collect::<Vec<_>>()[..] else {
                panic!("a fixture row has four columns: {l}");
            };
            Row {
                pod: pod.to_owned(),
                arrival: arrival.parse().expect("an arrival in ms"),
                line: oxikube_domain::log::LogLine::new(
                    ts.parse().expect("a timestamp"),
                    pod,
                    "app",
                    text,
                ),
            }
        })
        .collect();
    let expected: Vec<String> = include_str!("fixtures/three-pods.merged")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    assert_eq!(expected.len(), rows.len());
    let pods = ["api-a", "api-b", "api-c"];

    // Every rotation of which pod gets which extra latency: the order does not move.
    for turn in 0..pods.len() {
        let mut h = Harness::new();
        h.seed([deployment_named("api")]);
        for pod in pods {
            h.resources.insert(pod_named(pod));
        }
        for (i, pod) in pods.iter().enumerate() {
            let extra = ms(40 * ((i + turn) % pods.len()) as u64);
            let timeline = rows
                .iter()
                .filter(|r| r.pod == *pod)
                .fold(Timeline::new(), |t, r| {
                    t.ok_at(ms(r.arrival) + extra, r.line.clone())
                })
                .keep_open();
            h.script(timeline);
        }
        let spec = AggregateSpec::selector("default", "app=api");
        let session = h.open(spec, LogOptions::follow());
        h.run_for(ms(1_500));
        let merged: Vec<String> = texts(&session);
        assert_eq!(merged, expected, "latencies rotated by {turn}");
    }
}

fn deployment_named(name: &str) -> oxikube_domain::Resource {
    oxikube_testkit::deployment()
        .name(name)
        .namespace("default")
        .build()
}

fn pod_named(name: &str) -> oxikube_domain::Resource {
    oxikube_testkit::pod()
        .name(name)
        .namespace("default")
        .uid(format!("uid-{name}"))
        .label("app", "api")
        .build()
}
