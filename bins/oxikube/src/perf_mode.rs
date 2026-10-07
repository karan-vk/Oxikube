//! `oxikube --perf`: record a real run and print p50/p95/p99 on exit.
//!
//! The recorder is installed process-wide, the window root is wrapped in
//! `oxikube_runtime::perf::PerfRoot` (frame hook), and a background thread appends JSONL. The
//! session ends, printing the summary to stderr, on whichever comes first: the app quitting (last
//! window closed: `--perf` sets `QuitMode::LastWindowClosed`, also on macOS), `--perf-duration`
//! elapsing, or Ctrl-C (SIGINT).
//!
//! The summary ends with one line per connected cluster: its watch budget's counters (feeds,
//! objects, events, bytes, degrades, refusals, evictions; E04-F543), read from
//! `oxikube::kube_ports::WatchBudgets` when the session ends.
//!
//! `--perf-table <CONTEXT>` (`oxikube::perf_table`) makes the run a scripted one: it connects the
//! context, opens its pods table and scrolls it, the way a user would (E07-S09).

use anyhow::{Context as _, Result};
use gpui::{App, QuitMode};
use oxikube_runtime::perf::{self, DEFAULT_FLUSH_INTERVAL, PerfSession, Recorder};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

static SESSION: Mutex<Option<PerfSession>> = Mutex::new(None);

/// The watch-budget lines printed with the summary (set by [`attach`] once the app state exists).
type FeedReport = Box<dyn Fn() -> Vec<String> + Send>;
static FEEDS: Mutex<Option<FeedReport>> = Mutex::new(None);

/// Exit status after Ctrl-C (128 + SIGINT), as a shell would report it.
const SIGINT_EXIT: i32 = 130;

/// Starts the session before the GPUI app runs. Returns the recorder the window root reports to.
pub fn start(dir: Option<PathBuf>) -> Result<Arc<Recorder>> {
    let dir = dir
        .or_else(perf::default_dir)
        .context("the OS reports no data directory; pass --perf-dir")?;
    let recorder = Arc::new(Recorder::new());
    let session = PerfSession::start(
        recorder.clone(),
        &dir,
        DEFAULT_FLUSH_INTERVAL,
        env!("CARGO_PKG_VERSION"),
    )
    .with_context(|| format!("creating the perf log under {}", dir.display()))?;
    eprintln!("oxikube --perf: recording to {}", session.path().display());
    perf::install(recorder.clone());
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
    watch_ctrl_c();
    Ok(recorder)
}

/// Hooks the session end into the app: quit when the window closes, finish on quit, and quit
/// after `duration` if given.
pub fn attach(cx: &mut App, duration: Option<Duration>) {
    report_feeds(cx);
    cx.set_quit_mode(QuitMode::LastWindowClosed);
    cx.on_app_quit(|_| {
        finish();
        async {}
    })
    .detach();
    if let Some(duration) = duration {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(duration).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    }
}

/// Ends the session (idempotent): final drain, summary line, p50/p95/p99 to stderr.
pub fn finish() {
    let Some(session) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let finished = session.finish();
    eprintln!("oxikube --perf: {}", finished.summary);
    if let Some(report) = FEEDS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        for line in report() {
            eprintln!("oxikube --perf: {line}");
        }
    }
    eprintln!("oxikube --perf: wrote {}", finished.path.display());
    if let Some(err) = finished.error {
        eprintln!("oxikube --perf: the log is incomplete: {err}");
    }
}

/// Sets up the watch-budget lines of the summary: every connected cluster's counters, named by
/// its session title.
fn report_feeds(cx: &App) {
    let Some(state) = oxikube::app_state::AppState::try_global(cx) else {
        return;
    };
    let budgets = state.ports().clusters.budgets.clone();
    let sessions = state.services().sessions.clone();
    let report: FeedReport = Box::new(move || {
        budgets
            .stats()
            .iter()
            .map(|stats| {
                let name = sessions
                    .get(&stats.cluster)
                    .map_or_else(|| stats.cluster.to_string(), |s| s.title().to_owned());
                oxikube::kube_ports::report_line(&name, stats)
            })
            .collect()
    });
    *FEEDS.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
}

/// Ctrl-C would otherwise kill the process without a summary. A dedicated thread with a
/// current-thread tokio runtime waits for SIGINT, finishes the session and exits.
fn watch_ctrl_c() {
    let spawned = std::thread::Builder::new()
        .name("oxikube-perf-sigint".into())
        .spawn(|| {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            if runtime.block_on(tokio::signal::ctrl_c()).is_ok() {
                finish();
                std::process::exit(SIGINT_EXIT);
            }
        });
    if let Err(err) = spawned {
        eprintln!("oxikube --perf: Ctrl-C will not print a summary ({err})");
    }
}
