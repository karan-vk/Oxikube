//! The usage poll and the section's colours (E13-S11): an answer that did not change never wakes
//! the panel, a missing session is waited for, and dropping the panel ends the polling.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, Task, TestAppContext};
use jiff::Timestamp;
use oxikube_app::allocation::UsageState;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{FakeMetricsPort, MetricsCall};
use oxikube_ui::chart::Tone;

use super::poll::{USAGE_INTERVAL, UsageSubject, UsageTarget, poll_usage};
use super::section::tone_of;
use oxikube_app::allocation::MeasuredUsage;
use oxikube_app::allocation::{BarBasis, BarModel};

struct Probe {
    port: Arc<FakeMetricsPort>,
    /// Whether a session exists to ask.
    ready: bool,
    /// Whose usage to ask.
    subject: UsageSubject,
    applied: Vec<UsageState>,
    _poll: Task<()>,
}

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

fn node_reading(cpu_nanocores: u64) -> MetricsOutcome {
    let ts: Timestamp = "2026-10-12T10:00:00Z".parse().unwrap();
    MetricsOutcome::Available(vec![MetricsSample::new(
        MetricsSubject::Node { node: "n1".into() },
        ts,
        None,
        cpu_nanocores,
        1 << 30,
    )])
}

fn start(cx: &mut TestAppContext, ready: bool) -> (Entity<Probe>, Arc<FakeMetricsPort>) {
    cx.update(oxikube_runtime::init_deterministic);
    let port = Arc::new(FakeMetricsPort::new());
    let held = port.clone();
    let probe = cx.new(|cx: &mut Context<Probe>| Probe {
        port: held,
        ready,
        subject: UsageSubject::Node("n1".into()),
        applied: Vec::new(),
        _poll: poll_usage(
            cx,
            |probe: &Probe, _| {
                probe.ready.then(|| UsageTarget {
                    port: probe.port.clone(),
                    cluster: cluster(),
                    subject: probe.subject.clone(),
                })
            },
            |probe, state, _| probe.applied.push(state),
        ),
    });
    (probe, port)
}

fn calls(port: &FakeMetricsPort) -> usize {
    port.recorded_calls()
        .iter()
        .filter(|c| matches!(c, MetricsCall::NodeMetrics(_)))
        .count()
}

fn elapse(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

#[gpui::test]
fn an_unchanged_answer_never_wakes_the_panel(cx: &mut TestAppContext) {
    let (probe, port) = start(cx, true);
    for _ in 0..3 {
        port.script()
            .node_metrics
            .push_ok(node_reading(500_000_000));
    }
    cx.run_until_parked();
    assert_eq!(calls(&port), 1, "asked at once");
    elapse(cx, USAGE_INTERVAL);
    elapse(cx, USAGE_INTERVAL);
    assert_eq!(calls(&port), 3, "asked every interval");
    probe.read_with(cx, |p, _| {
        assert_eq!(p.applied.len(), 1, "the same reading, applied once")
    });

    // A different reading is applied.
    port.script()
        .node_metrics
        .push_ok(node_reading(900_000_000));
    elapse(cx, USAGE_INTERVAL);
    probe.read_with(cx, |p, _| assert_eq!(p.applied.len(), 2));
}

#[gpui::test]
fn an_absence_is_an_answer_too_and_a_change_to_it_is_applied(cx: &mut TestAppContext) {
    let (probe, port) = start(cx, true);
    port.script()
        .node_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    port.script().node_metrics.push_ok(node_reading(1));
    cx.run_until_parked();
    elapse(cx, USAGE_INTERVAL);
    probe.read_with(cx, |p, _| {
        assert!(matches!(&p.applied[0], UsageState::Unavailable(t) if t.starts_with("metrics unavailable: ")));
        assert!(matches!(&p.applied[1], UsageState::Ready(_)));
    });
}

#[gpui::test]
fn without_a_session_nothing_is_asked_until_there_is_one(cx: &mut TestAppContext) {
    let (probe, port) = start(cx, false);
    port.script().node_metrics.push_ok(node_reading(1));
    cx.run_until_parked();
    elapse(cx, Duration::from_secs(3));
    assert_eq!(calls(&port), 0, "no session, no call");
    probe.update(cx, |p, _| p.ready = true);
    elapse(cx, Duration::from_millis(300));
    assert_eq!(
        calls(&port),
        1,
        "asked a quarter second after the session appeared"
    );
}

#[gpui::test]
fn dropping_the_panel_ends_the_polling(cx: &mut TestAppContext) {
    let (probe, port) = start(cx, true);
    for _ in 0..5 {
        port.script().node_metrics.push_ok(node_reading(1));
    }
    cx.run_until_parked();
    assert_eq!(calls(&port), 1);
    drop(probe);
    cx.run_until_parked();
    elapse(cx, USAGE_INTERVAL * 3);
    assert_eq!(calls(&port), 1, "nothing observes: nothing is asked");
}

#[test]
fn the_fill_turns_amber_and_red_with_how_full_it_is() {
    let bar = |fill: f32, full: bool| BarModel {
        basis: BarBasis::Requests,
        fill,
        limits_mark: None,
        usage: None,
        full,
    };
    assert_eq!(tone_of(&bar(0.0, false)), Tone::Good);
    assert_eq!(tone_of(&bar(0.69, false)), Tone::Good);
    assert_eq!(tone_of(&bar(0.7, false)), Tone::Warn);
    assert_eq!(tone_of(&bar(0.9, false)), Tone::Critical);
    assert_eq!(
        tone_of(&bar(0.2, true)),
        Tone::Critical,
        "at the capacity is red"
    );
}

#[gpui::test]
fn an_absence_is_asked_again_sooner_than_a_reading(cx: &mut TestAppContext) {
    let (_probe, port) = start(cx, true);
    port.script()
        .node_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::ApiUnavailable));
    port.script().node_metrics.push_ok(node_reading(1));
    cx.run_until_parked();
    assert_eq!(calls(&port), 1);
    elapse(cx, Duration::from_secs(6));
    assert_eq!(
        calls(&port),
        2,
        "a transient failure is retried in seconds, not 15 s"
    );
}

#[gpui::test]
fn another_subject_does_not_inherit_the_old_reading(cx: &mut TestAppContext) {
    let (probe, port) = start(cx, true);
    port.script()
        .node_metrics
        .push_ok(node_reading(500_000_000));
    // The namespace's answer reads exactly like the node's did.
    port.script()
        .pod_metrics
        .push_ok(MetricsOutcome::Available(Vec::new()));
    cx.run_until_parked();
    probe.read_with(cx, |p, _| assert_eq!(p.applied.len(), 1));
    probe.update(cx, |p, _| {
        p.subject = UsageSubject::Namespace("shop".into())
    });
    elapse(cx, USAGE_INTERVAL);
    probe.read_with(cx, |p, _| {
        assert!(matches!(p.applied[0], UsageState::Ready(_)));
        assert_eq!(
            p.applied[1],
            UsageState::Pending,
            "the old reading is dropped at once"
        );
        assert!(matches!(
            &p.applied[2],
            UsageState::Ready(MeasuredUsage {
                pods_reporting: 0,
                ..
            })
        ));
    });
}
