//! Start-up: the documented init order (Zed's `main.rs` pattern, E05-S09).
//!
//! Every crate that needs set-up exposes `pub fn init(cx: &mut App)`; this module is the one
//! place that decides the order they run in and builds the adapters that become the
//! [`AppState`](crate::app_state::AppState) ports. The order matters: settings must exist before
//! the theme reads the `theme` setting, the keymap and the theme before the component library
//! consumes them, the runtime before anything spawns Kubernetes work, and the first frame must
//! not wait for the disk or the network.
//!
//! | # | [`Stage`] | What runs | Why here |
//! |---|---|---|---|
//! | 1 | `Logging` | [`boot`]: `oxikube_logging::init` (rolling files, redaction), the panic hook | before GPUI exists, so every later stage can log and every panic leaves a crash file |
//! | 2 | `Runtime` | `oxikube_runtime::init` | nothing may spawn Kubernetes work before it |
//! | 3 | `Assets` | `Application::with_assets(oxikube_ui::Assets)` | GPUI fixes the asset source when the `Application` is built, so this happens in `main` just before `run` |
//! | 4 | `Settings` | `oxikube_settings::init`, then `oxikube_logging::follow` | everything below reads settings; the log filter follows the `log.filter` setting from here on |
//! | 5 | `Theme` | `oxikube_theme::init` | reads the `theme` setting and the system appearance |
//! | 6 | `Keymap` | `oxikube_keymap::init` | binds the layered key bindings; needs settings' config dir |
//! | 7 | `Ui` | `oxikube_ui::init`, `follow_active_theme` | component library and tokens; consumes the active theme |
//! | 8 | `StateDb` | build [`LazyState`](state_db::LazyState) and start its open on the background executor | construction is instant; the open (SQLite, migrations) must not block the first frame, so it is not awaited |
//! | 9 | `AppState` | `AppState::install` | refuses to install unless the runtime, settings, theme and keymap exist, which is what makes a wrong order an error |
//! | 10 | `Workspace` | `oxikube_workspace::init` | window menu, workspace actions, session basics |
//! | 11 | `Features` | [`features::FEATURES`], in order | feature crates register actions, settings, item builders |
//! | 12 | `KeymapRebind` | `oxikube_keymap::rebind` | crates that bound keys after stage 6 (the workspace's interim bindings, the component library) must not outrank the user's `keymap.json` |
//! | 13 | `Window` | open the main window | everything it shows is ready; layout restore and the state db continue in the background |
//!
//! Stages 1 and 3 run in `main` ([`boot`], `run_app`); 2 and 4 to 12 are [`init`]; 13 is in
//! `run_app`. [`init`] runs once: a second call returns [`StartupError::AlreadyInitialised`] and
//! changes nothing, so nothing registers twice.
//!
//! # Cost
//!
//! Each stage runs in a `tracing` span named `init` and logs `init stage done` with its elapsed
//! microseconds, and the costs are kept in a [`StartupReport`] global, so the cold-start budget
//! work (E05-S13, `docs/PERFORMANCE.md`) reads them instead of guessing. Nothing in a stage may
//! block on I/O beyond reading the small config files (settings, keymap, themes dir) and creating
//! the log file.
//!
//! # Files
//!
//! - [`boot`]: logging, panic hook, data directory (before GPUI).
//! - `env`: [`StartupEnv`], what differs between the app and tests.
//! - `stage`: [`Stage`], [`StartupReport`].
//! - `state_db`: the lazily opened state database.
//! - `features`: the feature crates' inits.
//! - `paths`: data directory and file locations.

mod boot;
mod env;
mod features;
mod paths;
mod stage;
mod state_db;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use gpui::{App, Global};
use oxikube_keymap::KeymapOptions;
use oxikube_settings::SettingsStore;

use crate::app_state::{AppPorts, AppState, AppStateError};

pub use boot::{Boot, boot, shutdown};
pub use env::{ConfigSource, PortsChoice, RuntimeChoice, StartupEnv};
pub use features::{FEATURES, Feature};
pub use stage::{Stage, StageTiming, StartupReport, time_after_init};

/// Why [`init`] stopped.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    /// [`init`] already ran in this app. Nothing was changed.
    #[error("start-up already ran in this app")]
    AlreadyInitialised,
    /// The OS refused to create the runtime's worker threads.
    #[error("cannot start the runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// The embedded default settings are invalid (a build error).
    #[error("cannot build the settings store: {0}")]
    Settings(#[source] oxikube_domain::OxiError),
    /// An `init` ran out of order.
    #[error(transparent)]
    AppState(#[from] AppStateError),
}

/// Marks that [`init`] has run in this app.
struct Initialised;

impl Global for Initialised {}

/// Runs stages 2 and 4 to 12 of the [module docs](self) in order. Stage 13 (the window) is the
/// caller's.
pub fn init(cx: &mut App, env: StartupEnv) -> Result<(), StartupError> {
    init_with_features(cx, env, FEATURES)
}

/// [`init`] with an explicit feature list (tests; the app passes [`FEATURES`]).
pub(crate) fn init_with_features(
    cx: &mut App,
    env: StartupEnv,
    features: &[Feature],
) -> Result<(), StartupError> {
    if cx.has_global::<Initialised>() {
        return Err(StartupError::AlreadyInitialised);
    }
    cx.set_global(Initialised);
    let mut report = env.earlier.clone();

    report.time(Stage::Runtime, || match env.runtime {
        RuntimeChoice::Tokio => oxikube_runtime::init(cx).map_err(StartupError::Runtime),
        RuntimeChoice::Deterministic => {
            oxikube_runtime::init_deterministic(cx);
            Ok(())
        }
    })?;

    report.time(Stage::Settings, || init_settings(cx, &env))?;
    report.time(Stage::Theme, || match &env.config {
        ConfigSource::UserDir => oxikube_theme::init(cx),
        ConfigSource::Dir(dir) => {
            oxikube_theme::init_with_dir(Some(&oxikube_theme::user_dir::themes_dir(dir)), cx)
        }
        ConfigSource::Memory => oxikube_theme::init_with_dir(None, cx),
    });
    report.time(Stage::Keymap, || match &env.config {
        ConfigSource::UserDir => oxikube_keymap::init(cx),
        ConfigSource::Dir(dir) => oxikube_keymap::init_with_dir(dir, KeymapOptions::default(), cx),
        ConfigSource::Memory => oxikube_keymap::init_with_text("", KeymapOptions::default(), cx),
    });
    report.time(Stage::Ui, || {
        oxikube_ui::init(cx);
        oxikube_ui::follow_active_theme(cx).detach();
    });

    let ports = report.time(Stage::StateDb, || build_ports(cx, &env.ports));
    report.time(Stage::AppState, || {
        AppState::new(ports, env.data_dir.clone()).install(cx)
    })?;

    report.time(Stage::Workspace, || oxikube_workspace::init(cx));
    features::run(cx, &mut report, features);
    report.time(Stage::KeymapRebind, || oxikube_keymap::rebind(cx));

    tracing::info!(
        total_ms = report.total().as_millis() as u64,
        "init order finished"
    );
    cx.set_global(report);
    Ok(())
}

fn init_settings(cx: &mut App, env: &StartupEnv) -> Result<(), StartupError> {
    match &env.config {
        ConfigSource::UserDir => oxikube_settings::init(cx),
        ConfigSource::Dir(dir) => oxikube_settings::init_with_dir(dir, cx),
        ConfigSource::Memory => {
            let store = SettingsStore::new(oxikube_assets::default_settings())
                .map_err(StartupError::Settings)?;
            cx.set_global(store);
        }
    }
    if let Some(handle) = env.log.clone() {
        oxikube_logging::follow(cx, handle).detach();
    }
    Ok(())
}

/// Builds the ports bundle. The SQLite adapter is constructed here (and only here) and starts
/// opening in the background; nothing waits for it.
fn build_ports(cx: &mut App, choice: &PortsChoice) -> AppPorts {
    match choice {
        PortsChoice::Provided(ports) => ports.clone(),
        PortsChoice::Sqlite(path) => {
            let state = state_db::LazyState::new(path.clone());
            state.start(cx).detach();
            AppPorts::new(Arc::new(state))
        }
    }
}
