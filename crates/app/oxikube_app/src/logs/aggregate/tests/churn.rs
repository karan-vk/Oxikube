//! Churn following (E08-S07) in a multi-pod session: a rollout's new pods are read from their
//! first line, the old pods end with their lines kept, and a broken stream reconnects without
//! duplicates.

use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_ports::LogOptions;
use oxikube_testkit::{LogCall, ScriptedFeed, TICK, Timeline};

use super::{Harness, line, texts, web, web_pod};
use crate::logs::{LogState, PodChange, SourceState};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// `(pod, tail_lines)` of every stream opened, in order.
fn opened(h: &Harness) -> Vec<(String, Option<i64>)> {
    h.logs
        .recorded_calls()
        .into_iter()
        .map(|LogCall::StreamLogs { pod, options, .. }| (pod, options.tail_lines))
        .collect()
}

#[test]
fn a_rollout_restart_follows_the_new_pods_from_their_first_line_and_keeps_the_old_lines() {
    let mut h = Harness::new();
    h.seed([web()]);
    // Tick 1: the rollout's new pods appear. Tick 2: the old ones are deleted.
    ScriptedFeed::new()
        .initial([web_pod("web-a"), web_pod("web-b")])
        .add(1, web_pod("web-c"))
        .add(1, web_pod("web-d"))
        .delete(2, web_pod("web-a"))
        .delete(2, web_pod("web-b"))
        .install(&h.resources);
    // The old pods write until they are stopped, a little after their deletion.
    for (pod, at) in [("web-a", 0), ("web-b", 10)] {
        h.script(
            Timeline::new()
                .ok_at(ms(0), line(pod, at, &format!("{pod} old")))
                .ok_at(ms(2_300), line(pod, at + 2_300, &format!("{pod} last"))),
        );
    }
    for (pod, at) in [("web-c", 1_500), ("web-d", 1_550)] {
        h.script(Timeline::immediate([line(pod, at, &format!("{pod} first"))]).keep_open());
    }
    let session = h.open(
        crate::logs::AggregateSpec::of(&super::web_ref()).unwrap(),
        LogOptions::follow().tail_lines(100),
    );
    h.run_for(TICK);
    h.run_for(TICK);
    h.run_for(Duration::from_secs(2));

    assert_eq!(
        opened(&h),
        [
            ("web-a".into(), Some(100)),
            ("web-b".into(), Some(100)),
            ("web-c".into(), None),
            ("web-d".into(), None),
        ],
        "the pods of the first list read the tail; the new ones all of their log"
    );
    let view = session.aggregate();
    let changes: Vec<(String, PodChange)> = view
        .events_after(None)
        .iter()
        .map(|e| (e.pod.to_string(), e.change))
        .collect();
    assert_eq!(
        changes,
        [
            ("web-c".into(), PodChange::Added),
            ("web-d".into(), PodChange::Added),
            ("web-a".into(), PodChange::Ended),
            ("web-b".into(), PodChange::Ended),
        ]
    );
    let lines = texts(&session);
    assert_eq!(
        lines,
        [
            "web-a old",
            "web-b old",
            "web-c first",
            "web-d first",
            "web-a last",
            "web-b last"
        ],
        "every line once, the old pods' kept"
    );
    assert_eq!(
        session.state(),
        LogState::Streaming,
        "the view keeps following"
    );
    let live: Vec<_> = view
        .sources()
        .iter()
        .filter(|s| s.state.is_live())
        .map(|s| s.pod.to_string())
        .collect();
    assert_eq!(live, ["web-c", "web-d"]);
}

#[test]
fn a_broken_stream_of_a_running_pod_reconnects_and_drops_the_replayed_overlap() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a")]);
    h.script(
        Timeline::new()
            .ok_at(ms(0), line("web-a", 0, "a0"))
            .ok_at(ms(0), line("web-a", 100, "a1"))
            .err_at(ms(400), OxiError::network("connection reset by peer")),
    );
    h.script(
        Timeline::immediate([
            line("web-a", 0, "a0"),
            line("web-a", 100, "a1"),
            line("web-a", 900, "a2"),
        ])
        .keep_open(),
    );
    let session = h.open_web();
    h.run_for(ms(450));
    let sources = session.aggregate().sources();
    assert!(
        matches!(
            sources[0].state,
            SourceState::Reconnecting { attempt: 1, max: 5 }
        ),
        "{:?}",
        sources[0].state
    );
    assert!(sources[0].state.is_live());
    h.run_for(ms(1_200));
    assert_eq!(
        session.aggregate().sources()[0].state,
        SourceState::Streaming
    );
    assert_eq!(texts(&session), ["a0", "a1", "a2"]);
}
