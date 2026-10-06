//! The `startup` scenario: a cold start through the real init order to the first interactive
//! frame, with the per-stage breakdown (E05-S13, docs/PERFORMANCE.md "Startup").
//!
//! It runs what `main` runs, on GPUI's headless platform: [`startup::boot_in`] (logging, panic
//! hook) into a scratch data directory, the platform (timed as the `assets` stage, which builds the
//! `Application` in the app), [`startup::init`] with a scratch config directory (the settings,
//! keymap and themes files are created and read like a first launch), then the main window behind
//! the startup placeholder with the cluster UI mounted (catalog home, hotbar, cluster tabs; E07-S00)
//! and the first-frame probe. Differences from the app, each because the
//! headless scheduler rejects foreign threads waking its tasks: no settings/keymap file watchers
//! (`ConfigSource::Dir`), and the state db is an in-memory fake (the SQLite open runs off the UI
//! thread in the app, so it is not on the path to the first frame; its cost is measured separately
//! as `state_db_open_ms`, after the frames).
//!
//! The sample fails (exit 1) when, at the first frame, the process holds an IPv4/IPv6 socket (no
//! network before the first frame) or a lazy service has started (nothing heavy before the first
//! frame). Budgets on the numbers are `cargo xtask perf`'s job.
//!
//! Metrics (ms): `first_frame_ms` (first line of `main` to the end of the update that drew the first
//! frame), `config_load_ms` (settings + theme + keymap on the main thread), `init_<stage>_ms` for
//! every stage that ran, `state_db_open_ms`, then the idle-redraw `frame_ms` / `draw_ms` and the
//! memory metrics of every frame-driving scenario.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use gpui::{AnyView, AnyWindowHandle, App, AppContext as _, HeadlessAppContext};
use oxikube::app_state::{AppPorts, ClusterAdapters};
use oxikube::startup::{
    self, ConfigSource, PortsChoice, RuntimeChoice, Stage, StartupEnv, StartupReport,
};
use oxikube_runtime::LazyServices;
use oxikube_runtime::perf::harness::{self, metric};
use oxikube_runtime::perf::{PerfRoot, Recorder, ScenarioSample, Summary, round_ms};
use oxikube_testkit::headless;
use startup::first_frame::ms;

use super::{FIRST_FRAME_MARKER, FRAMES, WINDOW_SIZE};

/// The startup scenario's own metrics (the stage breakdown is `init_<stage>_ms`).
mod name {
    /// Settings, theme and keymap load on the main thread.
    pub const CONFIG_LOAD_MS: &str = "config_load_ms";
    /// Opening (creating and migrating) the SQLite state db, off the UI thread in the app.
    pub const STATE_DB_OPEN_MS: &str = "state_db_open_ms";
}

/// One sample. `probe` puts the `--perf` frame hook in the window, as `oxikube --perf` does.
pub fn run(launched: Instant, probe: bool) -> Result<ScenarioSample> {
    let scratch = Scratch::new()?;
    let result = measure(launched, probe, &scratch.0);
    startup::shutdown();
    result
}

fn measure(launched: Instant, probe: bool, scratch: &Path) -> Result<ScenarioSample> {
    let boot = startup::boot_in(launched, Some(scratch.join("data")));
    let mut earlier = boot.report;
    let mut cx = earlier.time(Stage::Assets, || {
        headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets))
    });
    let env = StartupEnv {
        config: ConfigSource::Dir(scratch.join("config")),
        runtime: RuntimeChoice::Tokio,
        ports: PortsChoice::Provided(fake_ports()),
        data_dir: boot.data_dir,
        log: boot.log,
        earlier,
    };
    std::fs::create_dir_all(scratch.join("config"))?;
    cx.update(|cx| startup::init(cx, env))?;
    let layout = cx.update(|cx| startup::window::main_layout_store(cx))?;

    let recorder = Arc::new(Recorder::new());
    let hook = probe.then(|| recorder.clone());
    let opening = Instant::now();
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, move |window, cx| {
            // The app's window: the cluster UI mounted before the first frame (E07-S00).
            oxikube_workspace::window::build_root_mounted(
                window,
                cx,
                Some(layout),
                oxikube::mount::mount_main_window,
                move |content, cx| wrap(content, hook, cx),
            )
        })?
        .into();
    // A test-mode context draws dirty windows when an update flushes its effects: the first frame
    // was drawn (and the probe fired) inside `open_window`.
    let opened = opening.elapsed();
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{FIRST_FRAME_MARKER}")?;
    stdout.flush()?;
    drop(stdout);

    let report = cx.update(|cx| {
        cx.global_mut::<StartupReport>()
            .record(Stage::Window, opened);
        check_nothing_started(cx).map(|()| cx.global::<StartupReport>().clone())
    })?;
    let first_frame = report
        .first_frame()
        .context("the first-frame probe did not fire while opening the window")?;
    if let Some(sockets) = first_frame.inet_sockets.filter(|n| *n > 0) {
        bail!("{sockets} network socket(s) were open at the first frame (no network before it)");
    }
    let since_launch = first_frame
        .since_launch
        .context("the launch instant was not recorded")?;
    eprintln!(
        "oxikube startup: {}",
        startup::first_frame::summary(&report)
    );

    settle_kube_tasks(&mut cx)?;
    if probe && recorder.frames_recorded() == 0 {
        bail!("no frame was drawn while opening the window");
    }
    let mut reader = recorder.reader();
    reader.drain(&recorder); // the first frame is first_frame_ms, not frame_ms
    let run = harness::run_frames(
        &mut cx,
        window,
        FRAMES,
        &recorder,
        &mut reader,
        |cx| cx.run_until_parked(),
        |_, _, _| {},
    )?;

    let mut extra = vec![
        single(metric::FIRST_FRAME_MS, ms(since_launch)),
        single(name::CONFIG_LOAD_MS, ms(report.config_load())),
    ];
    for timing in report.timings() {
        extra.push(single(
            &format!("init_{}_ms", timing.stage),
            ms(timing.elapsed),
        ));
    }
    extra.push(single(
        name::STATE_DB_OPEN_MS,
        state_db_open_ms(&scratch.join("state.db"))?,
    ));
    Ok(run.into_sample("startup", extra))
}

/// The app's ports with in-memory fakes: the state db (see the module docs) and the cluster side
/// (an empty catalog, so nothing is read or connected).
fn fake_ports() -> AppPorts {
    let ports = oxikube_testkit::TestPorts::empty();
    AppPorts::new(ports.state.clone(), ClusterAdapters::fakes(&ports))
}

/// The window content as the app wraps it: the `--perf` hook (when probing), then the first-frame
/// probe.
fn wrap(content: AnyView, hook: Option<Arc<Recorder>>, cx: &mut App) -> AnyView {
    let content = match hook {
        Some(recorder) => cx.new(|_| PerfRoot::new(content, recorder)).into(),
        None => content,
    };
    startup::window::probe_first_frame(content, cx)
}

/// How long [`settle_kube_tasks`] waits for the mount's background reads.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the window's tasks until the work the mount queued on the Tokio bridge (the catalog and
/// hotbar reads, E07-S00) has finished and its results are applied. Those reads finish on Tokio's
/// own threads at a time the headless scheduler does not control: a result landing inside the
/// frame loop would redraw the window once more and add an extra frame to `frame_ms`. A Tokio task
/// wakes its GPUI awaiter before it leaves the alive count, so a count of 0 followed by
/// `run_until_parked` means every result has been applied.
fn settle_kube_tasks(cx: &mut HeadlessAppContext) -> Result<()> {
    let tokio = cx.update(|cx| oxikube_runtime::handle(cx));
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        let alive = tokio.as_ref().map_or(0, |h| h.metrics().num_alive_tasks());
        if alive == 0 {
            cx.run_until_parked();
            return Ok(());
        }
        if started.elapsed() > SETTLE_TIMEOUT {
            bail!("{alive} Tokio task(s) still running {SETTLE_TIMEOUT:?} after the first frame");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Fails when a deferred service started before the first frame.
fn check_nothing_started(cx: &App) -> Result<()> {
    let started = LazyServices::started(cx);
    if started.is_empty() {
        return Ok(());
    }
    let names: Vec<_> = started.iter().map(|s| s.name).collect();
    bail!(
        "lazy services started before the first frame: {}",
        names.join(", ")
    )
}

/// Opens (creates and migrates) a fresh SQLite state db, as the app does on first launch, on this
/// thread and outside GPUI. In the app it runs on the adapter's own thread.
fn state_db_open_ms(path: &Path) -> Result<f64> {
    let started = Instant::now();
    let db = futures::executor::block_on(oxikube_state_sqlite::SqliteState::open(path))
        .context("opening the state db")?;
    let elapsed = started.elapsed();
    drop(db);
    Ok(ms(elapsed))
}

fn single(name: &str, ms: f64) -> (String, Summary) {
    (name.to_owned(), Summary::single(round_ms(ms)))
}

/// A scratch directory for this process, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("oxikube-perf-startup-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
