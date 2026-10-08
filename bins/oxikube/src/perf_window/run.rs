//! A windowed run from start-up to the summary: the environment the app starts in, the frame
//! hook's tap, the display's refresh, the script, and the summary handed back to `main`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{AnyWindowHandle, App, AsyncApp, px, size};
use oxikube_runtime::perf::windowed::{self, Flow, Meter, RunInfo, WindowedSummary};

use super::driver::Driver;
use super::{Scenario, scenarios, world};
use crate::startup::window::{main_view, perf_root};
use crate::startup::{Boot, ConfigSource, PortsChoice, RuntimeChoice, StartupEnv};

/// The window size every scenario runs at (logical pixels): a 14" laptop's usual window.
pub const WINDOW_SIZE: (f32, f32) = (1440.0, 900.0);
/// How long to wait for the window to become the active one before measuring anyway.
const ACTIVE_DEADLINE: Duration = Duration::from_secs(10);
/// How long the display's refresh is sampled before the scenario.
const CALIBRATION: Duration = Duration::from_millis(600);

/// A pod to run the `terminal` scenario's shell in (`--perf-exec CONTEXT/NAMESPACE/POD`), from
/// the user's kubeconfig, instead of a local shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTarget {
    /// The kubeconfig context.
    pub context: String,
    /// The pod's namespace.
    pub namespace: String,
    /// The pod (its first container gets the shell).
    pub pod: String,
}

impl ExecTarget {
    /// Parses `CONTEXT/NAMESPACE/POD` (the context may hold slashes).
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.rsplitn(3, '/');
        let pod = parts.next().filter(|s| !s.is_empty())?;
        let namespace = parts.next().filter(|s| !s.is_empty())?;
        let context = parts.next().filter(|s| !s.is_empty())?;
        Some(Self {
            context: context.to_owned(),
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
        })
    }
}

/// What to run.
#[derive(Debug, Clone)]
pub struct WindowRun {
    /// The scenario.
    pub scenario: Scenario,
    /// The `terminal` scenario's pod, when it should not use a local shell.
    pub exec: Option<ExecTarget>,
}

/// The environment the app starts in for `run`: the embedded default settings (nothing of the
/// user's is read or written), the app's Tokio runtime, an in-memory state db, and the
/// scenario's synthetic clusters; or, for a `terminal` run in a pod, the app's own kube adapters
/// over the user's kubeconfig.
pub fn startup_env(run: &WindowRun, boot: Boot) -> StartupEnv {
    let ports = match &run.exec {
        Some(_) => PortsChoice::Sqlite(":memory:".into()),
        None => {
            let spec = run.scenario.world();
            PortsChoice::Build(Arc::new(move |runtime| world::app_ports(&spec, runtime)))
        }
    };
    StartupEnv {
        config: ConfigSource::Memory,
        runtime: RuntimeChoice::Tokio,
        ports,
        data_dir: None,
        log: boot.log,
        earlier: boot.report,
    }
}

/// Runs `run` in `window` (the app's main window, opened with the `--perf` frame hook), then
/// calls `done` with the summary, or with why the script could not run.
pub fn start(
    run: WindowRun,
    window: AnyWindowHandle,
    done: impl FnOnce(Result<WindowedSummary>, &mut App) + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = drive(&run, window, cx).await;
        cx.update(|cx| done(result, cx));
    })
    // Detached on purpose: it lives until the scenario ends, then quits the app.
    .detach();
}

async fn drive(
    run: &WindowRun,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> Result<WindowedSummary> {
    let meter = Meter::new();
    let tap = meter.tap();
    let workspace = window.update(cx, |_, window, cx| {
        let hook = perf_root(window, cx).context("the window has no --perf frame hook")?;
        hook.update(cx, |hook, _| hook.set_tap(Some(tap)));
        let (width, height) = WINDOW_SIZE;
        window.resize(size(px(width), px(height)));
        main_view(window, cx)
            .map(|main| main.read(cx).workspace().clone())
            .context("the window is not the app's main window")
    })??;
    let mut notes = Vec::new();
    if !wait_active(window, cx).await {
        notes.push(format!(
            "the window did not become the active window within {ACTIVE_DEADLINE:?}"
        ));
    }
    let (refresh, source) = calibrate(window, cx).await?;
    if refresh.as_secs_f64() > 1.0 / 110.0 {
        notes.push(format!(
            "the display refreshed every {:.2} ms ({:.0} Hz), not at 120 Hz: dropped frames are \
             counted against it; the frame budget stays 8.33 ms",
            refresh.as_secs_f64() * 1000.0,
            1.0 / refresh.as_secs_f64()
        ));
    }
    // The size the scenario starts at (a script may resize the window on the way).
    let (viewport, scale) = window.update(cx, |_, window, _| {
        (window.viewport_size(), window.scale_factor())
    })?;
    notes.push(format!(
        "window {:.0} x {:.0} pt at scale {scale}",
        f32::from(viewport.width),
        f32::from(viewport.height),
    ));
    let mut driver = Driver::new(cx, window, meter.clone(), workspace)?;
    scenarios::run(run, &mut driver).await?;
    notes.extend(driver.take_notes());
    let info = RunInfo {
        scenario: run.scenario.name().to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        refresh,
        refresh_source: source.to_owned(),
        budgets: run.scenario.budgets(),
    };
    Ok(meter.finish(&info, notes))
}

/// Waits until the window is the key window (GPUI paces an inactive one at 30 fps).
async fn wait_active(window: AnyWindowHandle, cx: &mut AsyncApp) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < ACTIVE_DEADLINE {
        let active = window
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false);
        if active {
            return true;
        }
        cx.background_executor()
            .timer(Duration::from_millis(20))
            .await;
    }
    false
}

/// The display's refresh interval: the median gap between the refreshes the window is called on
/// while nothing is drawn (so none is missed). Falls back to 120 Hz.
async fn calibrate(window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<(Duration, &'static str)> {
    let probe = Meter::new();
    windowed::drive(cx, window, &probe, "calibrate", CALIBRATION, |_, _, _| {
        Ok(Flow::Continue)
    })
    .await?;
    let info = RunInfo {
        scenario: "calibrate".into(),
        app_version: String::new(),
        refresh: Duration::from_nanos(8_333_333),
        refresh_source: String::new(),
        budgets: windowed::Budgets::default(),
    };
    let gaps = probe.finish(&info, Vec::new()).scripted.refresh_gap_ms;
    Ok(match gaps {
        Some(gaps) if gaps.count >= 10 => (Duration::from_secs_f64(gaps.p50 / 1000.0), "measured"),
        _ => (Duration::from_nanos(8_333_333), "assumed"),
    })
}

/// Where the summary goes by default: next to the JSONL, `<name>.<scenario>.summary.json`.
pub fn default_summary_path(jsonl: &std::path::Path, scenario: Scenario) -> PathBuf {
    let stem = jsonl
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "oxikube-perf".to_owned());
    jsonl.with_file_name(format!("{stem}.{}.summary.json", scenario.name()))
}
