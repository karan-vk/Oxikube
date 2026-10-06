//! [`AppState`]: the typed dependency container of the app (Zed's `AppState` pattern, E05-S09).
//!
//! `AppState` is a GPUI [`Global`] holding what views and services need to reach the outside
//! world, and typed views over the platform globals each crate's `init(cx)` installed:
//!
//! | Part | Where it lives | Read with |
//! |---|---|---|
//! | ports bundle | the state itself ([`AppPorts`]: `Arc<dyn Port>`s built by this binary) | [`AppState::ports`], [`AppState::state`] |
//! | state db | `ports.state`: the SQLite adapter, opened off the UI thread | [`AppState::state`] |
//! | settings | `oxikube_settings::SettingsStore` global | [`AppState::settings`] |
//! | theme | `oxikube_theme::ThemeRegistry` + `ActiveTheme` globals | [`AppState::theme_registry`], [`AppState::active_theme`] |
//! | keymap | `oxikube_keymap::KeymapStore` global | [`AppState::keymap`] |
//! | runtime | `oxikube_runtime`'s tokio bridge global | [`AppState::runtime_mode`], [`AppState::runtime_handle`] |
//!
//! The platform stores stay their own globals (they are replaced in place by hot reload, so a
//! copy held here would go stale); `AppState` hands out references to the current ones and
//! [`AppState::install`] refuses to install until all of them exist, which is what turns a wrong
//! init order into an error at start-up rather than a panic in the middle of a frame. There is no
//! service-locator map: every part is a named, typed field or accessor.
//!
//! Adapters are built by `startup` (the only place that names them) and arrive here as port
//! trait objects, so nothing that takes an `AppState` knows SQLite or the keychain. The
//! `SecretStorePort` is wired when its adapter exists (`AppPorts::secrets` is `None` until then).
//!
//! # Tests
//!
//! `AppState::test(cx)` (test builds and feature `test-support`) runs the real init order with testkit fakes and no OS
//! threads: an in-memory settings store, the deterministic runtime, no file watchers, a
//! `oxikube_testkit::FakeStatePort`. It is the same code path as the app
//! (`startup::init`), so a test that calls it exercises the order.

mod ports;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, Global};
use oxikube_keymap::KeymapStore;
use oxikube_ports::StatePort;
use oxikube_runtime::RuntimeMode;
use oxikube_settings::SettingsStore;
use oxikube_theme::{ActiveTheme, ThemeRegistry, ThemeTokens};

pub use ports::AppPorts;

/// Why [`AppState::install`] refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AppStateError {
    /// An `AppState` is already installed. The first one stays; nothing was replaced.
    #[error("AppState is already installed")]
    AlreadyInstalled,
    /// A platform global the state reads is not there yet: `init(cx)` ran out of order.
    #[error("AppState needs the {0} to be initialised first (see the init order in `startup`)")]
    NotInitialised(&'static str),
}

/// The dependency container. See the [module docs](self).
pub struct AppState {
    ports: AppPorts,
    data_dir: Option<PathBuf>,
}

struct GlobalAppState(Arc<AppState>);

impl Global for GlobalAppState {}

impl AppState {
    /// A state over `ports`. `data_dir` is where logs, crash reports and the state database live
    /// (`None` for tests and when the OS reports none). It is not installed yet: see
    /// [`AppState::install`].
    pub fn new(ports: AppPorts, data_dir: Option<PathBuf>) -> Self {
        Self { ports, data_dir }
    }

    /// Installs `self` as the global.
    ///
    /// Checks that the runtime bridge, the settings store, the theme registry and the keymap
    /// have been initialised ([`AppStateError::NotInitialised`] otherwise) and that no state is
    /// installed yet ([`AppStateError::AlreadyInstalled`]; the first stays in effect).
    pub fn install(self, cx: &mut App) -> Result<Arc<AppState>, AppStateError> {
        if cx.has_global::<GlobalAppState>() {
            return Err(AppStateError::AlreadyInstalled);
        }
        if oxikube_runtime::mode(cx).is_none() {
            return Err(AppStateError::NotInitialised("runtime"));
        }
        if !cx.has_global::<SettingsStore>() {
            return Err(AppStateError::NotInitialised("settings store"));
        }
        if !cx.has_global::<ThemeRegistry>() {
            return Err(AppStateError::NotInitialised("theme registry"));
        }
        if !cx.has_global::<KeymapStore>() {
            return Err(AppStateError::NotInitialised("keymap"));
        }
        let state = Arc::new(self);
        cx.set_global(GlobalAppState(state.clone()));
        Ok(state)
    }

    /// The installed state. Panics before [`AppState::install`] (a wiring bug, like reading a
    /// GPUI global that was never set).
    #[track_caller]
    pub fn global(cx: &App) -> Arc<AppState> {
        cx.global::<GlobalAppState>().0.clone()
    }

    /// The installed state, if any.
    pub fn try_global(cx: &App) -> Option<Arc<AppState>> {
        cx.try_global::<GlobalAppState>().map(|g| g.0.clone())
    }

    /// The ports bundle.
    pub fn ports(&self) -> &AppPorts {
        &self.ports
    }

    /// The state db port (`ports().state`). In the app it is the SQLite adapter, opened in the
    /// background; calls wait for the open, never the UI thread.
    pub fn state(&self) -> &Arc<dyn StatePort> {
        &self.ports.state
    }

    /// The directory of logs, crash reports and the state database, when there is one.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// The settings store (layered, hot reloaded). Read typed values with `T::get_global(cx)`.
    pub fn settings<'a>(&self, cx: &'a App) -> &'a SettingsStore {
        cx.global::<SettingsStore>()
    }

    /// The theme registry (bundled and user themes).
    pub fn theme_registry<'a>(&self, cx: &'a App) -> &'a ThemeRegistry {
        ThemeRegistry::global(cx)
    }

    /// The theme in effect now.
    pub fn active_theme(&self, cx: &App) -> Arc<ThemeTokens> {
        ActiveTheme::get(cx)
    }

    /// The keymap layers.
    pub fn keymap<'a>(&self, cx: &'a App) -> &'a KeymapStore {
        cx.global::<KeymapStore>()
    }

    /// Which backend `spawn_kube` runs on.
    pub fn runtime_mode(&self, cx: &App) -> Option<RuntimeMode> {
        oxikube_runtime::mode(cx)
    }

    /// The tokio handle (`None` in the deterministic test runtime).
    pub fn runtime_handle(&self, cx: &App) -> Option<tokio::runtime::Handle> {
        oxikube_runtime::handle(cx)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AppState {
    /// The installed state, or one built by running the real init order on test fakes (see the
    /// [module docs](self)). Safe to call again in the same app: it returns the installed one.
    pub fn test(cx: &mut App) -> Arc<AppState> {
        if let Some(state) = Self::try_global(cx) {
            return state;
        }
        crate::startup::init(cx, crate::startup::StartupEnv::test())
            .expect("the test init order runs");
        Self::global(cx)
    }
}
