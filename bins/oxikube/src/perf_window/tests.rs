//! The windowed scenarios' scripts on GPUI's test platform, over the synthetic clusters (no churn,
//! a fake clock: nothing runs off GPUI's executors): the real init order, the main window with the
//! `--perf` frame hook, the script driven refresh by refresh, and the summary it hands back.
//! Phases run a tenth of their refreshes here (`TEST_TIME_PERCENT`); the app never shortens one.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui::{AppContext as _, TestAppContext, VisualTestContext};
use oxikube_runtime::perf::windowed::{PhaseKind, WindowedSummary};
use oxikube_runtime::perf::{PerfRoot, Recorder};
use oxikube_testkit::FakeClockPort;

use super::driver::TEST_TIME_PERCENT;
use super::run::{ExecTarget, default_summary_path};
use super::{Scenario, WindowRun, start, world};
use crate::startup::{
    ConfigSource, PortsChoice, RuntimeChoice, StartupEnv, StartupReport, init, window,
};

type Outcome = Rc<RefCell<Option<Result<WindowedSummary>>>>;

/// Starts `scenario` in a test window and drives it refresh by refresh until it hands back its
/// summary.
fn run(cx: &mut TestAppContext, scenario: Scenario) -> Result<WindowedSummary> {
    TEST_TIME_PERCENT.store(10, std::sync::atomic::Ordering::Relaxed);
    let ports = world::ports_with(&scenario.world(), Arc::new(FakeClockPort::default()), None);
    let env = StartupEnv {
        config: ConfigSource::Memory,
        runtime: RuntimeChoice::Deterministic,
        ports: PortsChoice::Provided(ports),
        data_dir: None,
        log: None,
        earlier: StartupReport::default(),
    };
    cx.update(|cx| init(cx, env)).expect("the init order runs");
    let recorder = Arc::new(Recorder::new());
    let handle = cx
        .update(|cx| {
            window::open_main_window(cx, move |content, cx| {
                cx.new(|_| PerfRoot::new(content, recorder)).into()
            })
        })
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();
    let outcome: Outcome = Rc::default();
    let slot = outcome.clone();
    let run = WindowRun {
        scenario,
        exec: None,
    };
    vcx.update(|_, cx| {
        start(
            run,
            handle.into(),
            move |result, _| *slot.borrow_mut() = Some(result),
            cx,
        );
    });
    for _ in 0..20_000 {
        vcx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        vcx.executor().advance_clock(Duration::from_micros(8_333));
        vcx.run_until_parked();
        if let Some(result) = outcome.borrow_mut().take() {
            return result;
        }
    }
    panic!("the scenario did not end");
}

fn phase<'a>(
    summary: &'a WindowedSummary,
    name: &str,
) -> &'a oxikube_runtime::perf::windowed::PhaseSummary {
    summary
        .phases
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no phase {name}: {:?}", summary.phases))
}

#[gpui::test]
fn the_pods_table_is_scrolled_by_scroll_events_and_left_still(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::PodsTable).expect("the script ran");
    assert_eq!(summary.scenario, "pods-table");
    let names: Vec<&str> = summary.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["setup", "scroll", "still"]);
    let scroll = phase(&summary, "scroll");
    assert_eq!(scroll.kind, PhaseKind::Driven);
    assert!(scroll.refreshes > 0 && scroll.inputs > 0, "{scroll:?}");
    assert!(scroll.frames.is_some(), "the hook's tap fed the meter");
    assert_eq!(phase(&summary, "still").kind, PhaseKind::Idle);
    assert!(
        summary.budgets.peak_rss_mib.is_some(),
        "10 000 pods carry the memory budget"
    );
    assert!(summary.notes.iter().any(|n| n.starts_with("window ")));
}

#[gpui::test]
fn the_catalog_search_is_typed_key_by_key(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::Catalog).expect("typing narrowed the catalog");
    let typing = phase(&summary, "type-search");
    assert!(typing.inputs > 0);
    assert!(
        typing.input_latency_ms.is_some(),
        "every key reached a frame"
    );
}

/// The scripts that act through commands, actions and keys reach their view (each fails its run
/// otherwise: a filter that does not filter, a tab key that does not switch).
#[gpui::test]
fn the_table_filter_is_typed_into_the_table(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::TableFilter).expect("typing filtered the table");
    assert!(phase(&summary, "type-filter").inputs > 0);
}

#[gpui::test]
fn the_namespace_is_switched_through_the_bus(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::Namespaces).expect("switching narrowed the table");
    assert!(phase(&summary, "switch-namespace").inputs > 0);
}

#[gpui::test]
fn the_theme_is_switched_through_the_settings(cx: &mut TestAppContext) {
    run(cx, Scenario::Theme).expect("the theme changed");
}

#[gpui::test]
fn the_drawer_tabs_are_cycled_with_their_keys(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::DetailDrawer).expect("every tab was shown");
    assert!(phase(&summary, "cycle-tabs").inputs >= 4);
}

#[gpui::test]
fn cluster_tabs_switch_and_the_window_and_dock_resize(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::TabsPanes).expect("the tabs switched");
    let names: Vec<&str> = summary.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["setup", "switch-tabs", "resize-window", "resize-dock"]
    );
}

#[gpui::test]
fn two_idle_clusters_are_an_idle_phase(cx: &mut TestAppContext) {
    let summary = run(cx, Scenario::Idle).expect("two clusters connected");
    let idle = phase(&summary, "idle");
    assert_eq!(idle.kind, PhaseKind::Idle);
    assert!(
        idle.activity_checks > 0,
        "an idle phase checks that its window stays active: {idle:?}"
    );
    assert_eq!(
        summary.valid,
        idle.inactive_checks == 0,
        "an idle-only scenario is a measurement while its window stays active: {:?}",
        summary.notes
    );
}

#[test]
fn every_scenario_has_a_name_a_world_and_the_frame_budget() {
    for scenario in Scenario::ALL {
        assert_eq!(Scenario::parse(scenario.name()), Some(scenario));
        assert!((scenario.budgets().frame_ms - 1000.0 / 120.0).abs() < 1e-9);
        let world = scenario.world();
        match scenario {
            Scenario::Terminal => assert!(world.clusters.is_empty()),
            Scenario::Catalog => {
                assert_eq!(world.clusters.len() + world.listed_only, 50, "50 contexts");
            }
            Scenario::TabsPanes => assert_eq!(world.clusters.len(), 3),
            Scenario::PodsTable
            | Scenario::TableFilter
            | Scenario::Namespaces
            | Scenario::Theme
            | Scenario::Sidebar => {
                assert_eq!(world.clusters.len(), 1);
                assert_eq!(world.clusters[0].pods, super::PODS);
                assert_eq!(
                    scenario.budgets().peak_rss_mib,
                    Some(super::MEMORY_10K_PODS_MIB),
                    "{} lists 10 000 pods and nothing else",
                    scenario.name()
                );
            }
            Scenario::Idle => {
                assert_eq!(world.clusters.len(), 2);
                assert!(world.clusters.iter().all(|c| !c.churn));
                assert_eq!(scenario.budgets().idle_cpu_percent, Some(1.0));
                assert_eq!(
                    scenario.budgets().peak_rss_mib,
                    Some(super::MEMORY_IDLE_MIB),
                    "idle is judged on the 150 MB two-cluster idle memory budget"
                );
            }
            _ => {
                assert_eq!(world.clusters[0].pods, super::PODS);
                assert!(world.clusters[0].churn);
            }
        }
    }
    assert_eq!(Scenario::parse("nope"), None);
    assert!((super::MEMORY_10K_PODS_MIB - 381.47).abs() < 0.01);
    assert!((super::MEMORY_IDLE_MIB - 143.05).abs() < 0.01);
}

#[test]
fn exec_targets_and_summary_paths() {
    let target = ExecTarget::parse("arn:aws:eks/x/ns/tty").unwrap();
    assert_eq!(
        (
            target.context.as_str(),
            target.namespace.as_str(),
            target.pod.as_str()
        ),
        ("arn:aws:eks/x", "ns", "tty")
    );
    assert!(ExecTarget::parse("ns/tty").is_none());
    let path = default_summary_path(
        std::path::Path::new("/p/oxikube-perf-1-2.jsonl"),
        Scenario::Logs,
    );
    assert_eq!(
        path,
        std::path::Path::new("/p/oxikube-perf-1-2.logs.summary.json")
    );
}
