//! The meter's arithmetic on scripted frames, refreshes and inputs, and the pacer on a test window.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Context, IntoElement, Render, TestAppContext, Window, div};

use super::*;
use crate::perf::{FrameNotifies, FrameSample, PerfRoot, Recorder};

const REFRESH: Duration = Duration::from_nanos(8_333_333);

fn info(budgets: Budgets) -> RunInfo {
    RunInfo {
        scenario: "test".into(),
        app_version: "0.0.0".into(),
        refresh: REFRESH,
        refresh_source: "assumed".into(),
        budgets,
    }
}

fn frame(meter: &Meter, start: Instant, ms: f64, views: u64) {
    meter.frame(FrameSample {
        start,
        duration: Duration::from_secs_f64(ms / 1000.0),
        notifies: FrameNotifies {
            total: views,
            max_per_view: views.min(1),
        },
    });
}

#[test]
fn a_gap_of_more_than_one_and_a_half_refreshes_drops_the_ones_between() {
    assert_eq!(missed_refreshes(REFRESH, REFRESH), 0);
    assert_eq!(missed_refreshes(REFRESH.mul_f64(1.4), REFRESH), 0, "jitter");
    assert_eq!(missed_refreshes(REFRESH.mul_f64(1.6), REFRESH), 1);
    assert_eq!(missed_refreshes(REFRESH * 2, REFRESH), 1);
    assert_eq!(missed_refreshes(REFRESH * 5, REFRESH), 4);
    assert_eq!(missed_refreshes(REFRESH * 5, Duration::ZERO), 0);
}

#[test]
fn setup_is_reported_but_only_scripted_phases_are_judged() {
    let meter = Meter::new();
    let t0 = Instant::now();
    // A slow first frame while setting up: reported, not judged.
    frame(&meter, t0, 40.0, 0);
    meter.begin("scroll", PhaseKind::Driven);
    for i in 0..10u32 {
        let at = t0 + REFRESH * (i + 10);
        meter.refresh(at, true);
        meter.input();
        // The input is marked on the real clock: its frame starts right after it.
        frame(&meter, Instant::now(), 3.0, 1);
    }
    let s = meter.finish(&info(Budgets::default()), Vec::new());
    assert!(s.valid);
    assert!(s.passed(), "{:?}", s.failures);
    assert_eq!(s.phases.len(), 2);
    assert_eq!(s.phases[0].name, "setup");
    assert_eq!(s.phases[0].frames.unwrap().max, 40.0);
    let scripted = &s.scripted;
    assert_eq!(scripted.frames.unwrap().count, 10);
    assert_eq!((scripted.refreshes, scripted.dropped_frames), (10, 0));
    assert_eq!(scripted.inputs, 10);
    assert_eq!(scripted.input_latency_ms.unwrap().count, 10);
    let latency = scripted.input_latency_ms.unwrap();
    assert!(latency.max >= 3.0 && latency.max < 8.0, "{latency:?}");
    assert!(s.over_budget.is_empty());
}

#[test]
fn a_slow_frame_a_missed_refresh_and_a_late_input_each_fail_the_run() {
    let meter = Meter::new();
    let t0 = Instant::now();
    frame(&meter, t0, 1.0, 0);
    meter.begin("type", PhaseKind::Driven);
    meter.refresh(t0 + REFRESH, true);
    meter.input();
    frame(&meter, t0 + REFRESH, 12.0, 1);
    // The 12 ms frame made the next refresh come two intervals later.
    meter.refresh(t0 + REFRESH * 3, true);
    meter.input();
    // No frame for that input before the next one.
    meter.refresh(t0 + REFRESH * 4, true);
    meter.input();
    frame(&meter, t0 + REFRESH * 4, 2.0, 1);
    let s = meter.finish(&info(Budgets::default()), Vec::new());
    assert!(s.valid);
    let scripted = &s.scripted;
    assert_eq!(scripted.over_budget_frames, 1);
    assert_eq!(scripted.dropped_frames, 1);
    assert_eq!(scripted.inputs_without_frame, 1);
    assert_eq!(s.over_budget.len(), 1);
    assert_eq!(s.over_budget[0].phase, "type");
    assert_eq!(s.over_budget[0].ms, 12.0);
    assert!(s.over_budget[0].t_ms > 8.0, "since the first frame");
    let failures = s.failures.join(" | ");
    for expected in [
        "frames over",
        "refreshes dropped",
        "input latency",
        "never reached a frame",
    ] {
        assert!(failures.contains(expected), "{expected}: {failures}");
    }
    assert!(!s.passed());
}

#[test]
fn an_inactive_window_makes_the_run_no_measurement() {
    let meter = Meter::new();
    meter.begin("scroll", PhaseKind::Driven);
    let t0 = Instant::now();
    meter.refresh(t0, true);
    meter.refresh(t0 + REFRESH * 4, false);
    let s = meter.finish(&info(Budgets::default()), Vec::new());
    assert!(!s.valid);
    assert!(!s.passed());
    assert!(s.notes.iter().any(|n| n.contains("not the active window")));
    assert!(
        s.lines()
            .last()
            .is_some_and(|l| l.contains("NOT A MEASUREMENT"))
    );
}

#[test]
fn two_notifies_for_one_view_in_a_frame_fail_the_coalescing_budget() {
    let meter = Meter::new();
    meter.begin("stream", PhaseKind::Driven);
    let t0 = Instant::now();
    meter.refresh(t0, true);
    meter.frame(FrameSample {
        start: t0,
        duration: Duration::from_millis(1),
        notifies: FrameNotifies {
            total: 3,
            max_per_view: 2,
        },
    });
    let s = meter.finish(&info(Budgets::default()), Vec::new());
    assert_eq!(s.scripted.max_notifies_per_frame, 3);
    assert_eq!(s.scripted.max_view_notifies_per_frame, 2);
    assert!(s.failures.iter().any(|f| f.contains("coalesced notifies")));
}

#[test]
fn idle_cpu_and_memory_budgets_apply_where_set() {
    let meter = Meter::new();
    meter.begin("scroll", PhaseKind::Driven);
    meter.refresh(Instant::now(), true);
    meter.begin("idle", PhaseKind::Idle);
    let started = Instant::now();
    let mut x = 0u64;
    while started.elapsed() < Duration::from_millis(30) {
        x = std::hint::black_box(x.wrapping_add(1));
    }
    let budgets = Budgets {
        peak_rss_mib: Some(1.0),
        idle_cpu_percent: Some(1.0),
        ..Budgets::default()
    };
    let s = meter.finish(&info(budgets), Vec::new());
    let idle = s.phases.iter().find(|p| p.name == "idle").unwrap();
    assert_eq!(idle.kind, PhaseKind::Idle);
    if cfg!(unix) {
        assert!(idle.cpu_percent.unwrap() > 50.0, "spinning is busy");
        assert!(s.failures.iter().any(|f| f.contains("idle: CPU")));
    }
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        assert!(idle.peak_rss_mib.is_some());
        assert!(s.failures.iter().any(|f| f.contains("peak RSS")));
    }
}

#[test]
fn the_summary_round_trips_through_json() {
    let meter = Meter::new();
    meter.begin("scroll", PhaseKind::Driven);
    meter.refresh(Instant::now(), true);
    let s = meter.finish(&info(Budgets::default()), vec!["a note".into()]);
    let json = serde_json::to_string(&s).unwrap();
    let back: WindowedSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.schema, WINDOWED_SCHEMA);
}

struct Counter(Rc<Cell<u32>>);

impl Render for Counter {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div()
    }
}

/// The pacer calls the step once per refresh until the step stops, every refresh is counted, and
/// the frames the hook draws land in the phase.
#[gpui::test]
fn drive_steps_once_per_refresh_and_the_hook_feeds_the_meter(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let recorder = Arc::new(Recorder::new());
    let meter = Meter::new();
    let tap = meter.tap();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            let inner = cx.new(|_| Counter(renders.clone()));
            cx.new(|_| {
                let mut root = PerfRoot::new(inner, recorder.clone());
                root.set_tap(Some(tap));
                root
            })
        })
        .unwrap()
    });
    let handle = window.into();
    let steps = Rc::new(Cell::new(0u64));
    let seen = steps.clone();
    let driving = {
        let meter = meter.clone();
        cx.spawn(move |mut cx| async move {
            drive(
                &mut cx,
                handle,
                &meter,
                "steps",
                Duration::from_secs(3600),
                move |step, window, _| {
                    seen.set(seen.get() + 1);
                    step.input();
                    window.refresh();
                    Ok(if step.index == 4 {
                        Flow::Stop
                    } else {
                        Flow::Continue
                    })
                },
            )
            .await
        })
    };
    cx.run_until_parked();
    for _ in 0..10 {
        cx.update_window(handle, |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    let result = futures::FutureExt::now_or_never(driving).expect("the phase ended");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(steps.get(), 5, "one step per refresh until Stop");
    assert_eq!(meter.phase(), "steps");
    assert!(meter.frames_in_phase() >= 4, "{}", meter.frames_in_phase());
    let s = meter.finish(&info(Budgets::default()), Vec::new());
    assert_eq!(s.scripted.refreshes, 5);
    assert_eq!(s.scripted.inputs, 5);
}

/// `docs/perf/windowed-summary.example.json` is the contract with `cargo xtask perf --windowed`
/// (which parses it with its own mirror of the format).
#[test]
fn the_example_summary_is_this_format() {
    let text = include_str!("../../../../../../docs/perf/windowed-summary.example.json");
    let summary: WindowedSummary = serde_json::from_str(text).unwrap();
    assert_eq!(summary.schema, WINDOWED_SCHEMA);
    let back = serde_json::to_value(&summary).unwrap();
    let original: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        back, original,
        "every field of the example is a field of the format"
    );
}

/// A window that gets no refresh (hidden, minimised, another Space) fails the phase instead of
/// hanging the run.
#[gpui::test]
fn a_window_without_refreshes_fails_the_phase(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| Counter(renders.clone()))
        })
        .unwrap()
    });
    let handle = window.into();
    let meter = Meter::new();
    let driving = cx.spawn(move |mut cx| async move {
        drive(
            &mut cx,
            handle,
            &meter,
            "hidden",
            Duration::from_secs(3600),
            |_, _, _| Ok(Flow::Continue),
        )
        .await
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    let result = futures::FutureExt::now_or_never(driving).expect("the phase gave up");
    let err = result.expect_err("no refresh is an error");
    assert!(err.to_string().contains("no display refresh"), "{err}");
}
