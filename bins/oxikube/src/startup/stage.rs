//! The init stages, their order, and what each one cost.

use std::fmt;
use std::time::{Duration, Instant};

use gpui::{App, Global};

/// One step of start-up, in the order they run. The doc table in [`crate::startup`] says what each
/// does and why it sits where it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// `tracing` to rolling files, the panic hook (before GPUI exists).
    Logging,
    /// The asset source registered on the `Application` (before GPUI's `run`).
    Assets,
    /// The tokio <-> GPUI bridge (needs an `App`, so it is the first stage inside `run`).
    Runtime,
    /// The settings store, then the log filter following it.
    Settings,
    /// The theme registry and the active theme.
    Theme,
    /// The keymap layers.
    Keymap,
    /// `oxikube_ui`: the component library, tokens, theme bridge.
    Ui,
    /// The state database: built and started in the background, not waited for.
    StateDb,
    /// The `AppState` global.
    AppState,
    /// `oxikube_workspace`: window menu, actions, session basics.
    Workspace,
    /// Feature crates' `init(cx)`.
    Features,
    /// Re-binding the keymap on top of everything other crates bound.
    KeymapRebind,
    /// Opening the main window.
    Window,
}

impl Stage {
    /// Every stage, in start-up order.
    pub const ALL: [Stage; 13] = [
        Stage::Logging,
        Stage::Assets,
        Stage::Runtime,
        Stage::Settings,
        Stage::Theme,
        Stage::Keymap,
        Stage::Ui,
        Stage::StateDb,
        Stage::AppState,
        Stage::Workspace,
        Stage::Features,
        Stage::KeymapRebind,
        Stage::Window,
    ];

    /// The stage's name in logs and spans.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Logging => "logging",
            Stage::Runtime => "runtime",
            Stage::Assets => "assets",
            Stage::Settings => "settings",
            Stage::Theme => "theme",
            Stage::Keymap => "keymap",
            Stage::Ui => "ui",
            Stage::StateDb => "state_db",
            Stage::AppState => "app_state",
            Stage::Workspace => "workspace",
            Stage::Features => "features",
            Stage::KeymapRebind => "keymap_rebind",
            Stage::Window => "window",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How long a stage took on the calling thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageTiming {
    /// The stage.
    pub stage: Stage,
    /// Wall time it took.
    pub elapsed: Duration,
}

/// Per-stage costs of this start-up, in the order the stages ran, and when the first interactive
/// frame was drawn (E05-S13). Read it with [`StartupReport::get`].
#[derive(Debug, Clone, Default)]
pub struct StartupReport {
    timings: Vec<StageTiming>,
    launched: Option<Instant>,
    first_frame: Option<FirstFrame>,
}

/// The end of start-up: the main window's first interactive frame (see
/// [`crate::startup::first_frame`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstFrame {
    /// From the first line of `main` ([`StartupReport::launched_at`]) to the end of the update
    /// that drew the first frame; `None` when the launch instant is unknown.
    pub since_launch: Option<Duration>,
    /// IPv4/IPv6 sockets the process held at that moment: 0 means no network before the first
    /// frame. `None` where the OS offers no way to count them.
    pub inet_sockets: Option<usize>,
}

impl Global for StartupReport {}

impl StartupReport {
    /// An empty report for a process launched at `launched` (the first line of `main`).
    pub fn launched_at(launched: Instant) -> Self {
        Self {
            launched: Some(launched),
            ..Self::default()
        }
    }

    /// When the process started, if known.
    pub fn launched(&self) -> Option<Instant> {
        self.launched
    }

    /// The first interactive frame, once it has been drawn.
    pub fn first_frame(&self) -> Option<FirstFrame> {
        self.first_frame
    }

    /// Records the first interactive frame (only the first call counts).
    pub fn record_first_frame(&mut self, frame: FirstFrame) {
        self.first_frame.get_or_insert(frame);
    }

    /// What `stage` cost, when it ran.
    pub fn elapsed(&self, stage: Stage) -> Option<Duration> {
        self.timings
            .iter()
            .find(|t| t.stage == stage)
            .map(|t| t.elapsed)
    }

    /// The main-thread cost of loading the settings, theme and keymap (stages `Settings`, `Theme`
    /// and `Keymap`): the part of start-up with its own 30 ms budget (docs/PERFORMANCE.md).
    pub fn config_load(&self) -> Duration {
        [Stage::Settings, Stage::Theme, Stage::Keymap]
            .into_iter()
            .filter_map(|stage| self.elapsed(stage))
            .sum()
    }

    /// The report of this run; `None` before [`crate::startup::init`] finished.
    pub fn get(cx: &App) -> Option<&Self> {
        cx.try_global::<Self>()
    }

    /// The stages that ran, in order.
    pub fn timings(&self) -> &[StageTiming] {
        &self.timings
    }

    /// The stage names in the order they ran.
    pub fn order(&self) -> Vec<Stage> {
        self.timings.iter().map(|t| t.stage).collect()
    }

    /// The sum of every stage's cost.
    pub fn total(&self) -> Duration {
        self.timings.iter().map(|t| t.elapsed).sum()
    }

    /// Records `stage` as having taken `elapsed` (for a stage timed elsewhere).
    pub fn record(&mut self, stage: Stage, elapsed: Duration) {
        self.timings.push(StageTiming { stage, elapsed });
        tracing::info!(
            stage = stage.name(),
            elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
            "init stage done"
        );
    }

    /// Runs `f` as `stage`: inside a tracing span named `init`, timed, and recorded.
    pub fn time<R>(&mut self, stage: Stage, f: impl FnOnce() -> R) -> R {
        let started = Instant::now();
        let result = tracing::info_span!("init", stage = stage.name()).in_scope(f);
        self.record(stage, started.elapsed());
        result
    }
}

/// Runs `f` as `stage` and appends its cost to the installed report (a stage that happens after
/// [`crate::startup::init`], such as opening the window). Without a report it just runs `f`.
pub fn time_after_init<R>(cx: &mut App, stage: Stage, f: impl FnOnce(&mut App) -> R) -> R {
    let started = Instant::now();
    let result = tracing::info_span!("init", stage = stage.name()).in_scope(|| f(cx));
    if cx.has_global::<StartupReport>() {
        cx.global_mut::<StartupReport>()
            .record(stage, started.elapsed());
    }
    result
}
