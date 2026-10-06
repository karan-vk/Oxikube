//! The end of start-up: the main window's first interactive frame, and the budgets it is held to
//! (ADR 0013, docs/PERFORMANCE.md "Startup").
//!
//! The main window's content is wrapped in `oxikube_runtime::perf::FirstFrameProbe` (see
//! [`super::window`]), which calls [`mark`] at the end of the update that drew the first frame:
//! after `draw` and `present`, when the window's focus tree and hit boxes exist and input
//! dispatches. [`mark`] stores a [`FirstFrame`] in the [`StartupReport`], counts the network
//! sockets the process holds at that moment (the "no network before the first frame" rule) and
//! logs both; under `--perf` it also prints the start-up breakdown to stderr.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use gpui::App;
use oxikube_runtime::perf::sockets;

use super::stage::{FirstFrame, StartupReport};

/// Cold start to the first interactive frame (ADR 0013).
pub const STARTUP_BUDGET: Duration = Duration::from_millis(400);

/// Settings, theme and keymap load on the main thread ([`StartupReport::config_load`]).
pub const CONFIG_LOAD_BUDGET: Duration = Duration::from_millis(30);

/// Records the first interactive frame now. Called once by the probe; later calls change nothing.
pub fn mark(cx: &mut App) {
    let now = Instant::now();
    let inet_sockets = sockets::inet_socket_count();
    if !cx.has_global::<StartupReport>() {
        cx.set_global(StartupReport::default());
    }
    let report = cx.global_mut::<StartupReport>();
    if report.first_frame().is_some() {
        return;
    }
    let frame = FirstFrame {
        since_launch: report.launched().map(|launched| now - launched),
        inet_sockets,
    };
    report.record_first_frame(frame);
    let report = cx.global::<StartupReport>();
    log(report, frame);
    if oxikube_runtime::perf::enabled() {
        eprintln!("oxikube --perf: {}", summary(report));
    }
}

fn log(report: &StartupReport, frame: FirstFrame) {
    let since_launch_ms = frame.since_launch.map(ms);
    tracing::info!(
        since_launch_ms,
        config_load_ms = ms(report.config_load()),
        inet_sockets = frame.inet_sockets,
        "first interactive frame"
    );
    if frame.since_launch.is_some_and(|d| d > STARTUP_BUDGET) {
        tracing::warn!(
            since_launch_ms,
            budget_ms = ms(STARTUP_BUDGET),
            "start-up is over its budget"
        );
    }
    if report.config_load() > CONFIG_LOAD_BUDGET {
        tracing::warn!(
            config_load_ms = ms(report.config_load()),
            budget_ms = ms(CONFIG_LOAD_BUDGET),
            "settings, theme and keymap load is over its budget"
        );
    }
    if frame.inet_sockets.is_some_and(|n| n > 0) {
        tracing::warn!(
            inet_sockets = frame.inet_sockets,
            "network sockets were open before the first frame"
        );
    }
}

/// One line: time to the first frame, the config load, the sockets, then every stage's cost.
pub fn summary(report: &StartupReport) -> String {
    let mut line = String::new();
    match report.first_frame() {
        Some(FirstFrame {
            since_launch: Some(since),
            ..
        }) => {
            let _ = write!(
                line,
                "first interactive frame after {:.1} ms (budget {} ms)",
                ms(since),
                STARTUP_BUDGET.as_millis()
            );
        }
        _ => line.push_str("first interactive frame not timed"),
    }
    let _ = write!(
        line,
        "; settings+theme+keymap {:.1} ms (budget {} ms)",
        ms(report.config_load()),
        CONFIG_LOAD_BUDGET.as_millis()
    );
    match report.first_frame().and_then(|f| f.inet_sockets) {
        Some(n) => {
            let _ = write!(line, "; network sockets before it: {n}");
        }
        None => line.push_str("; network sockets not counted on this OS"),
    }
    line.push_str("; init:");
    for timing in report.timings() {
        let _ = write!(line, " {} {:.2}", timing.stage, ms(timing.elapsed));
    }
    // What no stage covers: the platform's run loop starting up before `run` calls back, and the
    // window and its first draw when those finish before the window stage is recorded.
    if let Some(since) = report.first_frame().and_then(|f| f.since_launch) {
        let _ = write!(
            line,
            " other {:.2}",
            ms(since.saturating_sub(report.total()))
        );
    }
    line.push_str(" ms");
    line
}

/// Milliseconds with microsecond precision.
pub fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}
