//! [`AppState`]: the typed dependency container of the app (Zed's `AppState` pattern, E05-S09).
//!
//! `AppState` is a GPUI [`Global`] holding what views and services need to reach the outside
//! world, and typed views over the platform globals each crate's `init(cx)` installed:
//!
//! | Part | Where it lives | Read with |
//! |---|---|---|
//! | ports bundle | the state itself ([`AppPorts`]: `Arc<dyn Port>`s built by this binary, the cluster adapters among them) | [`AppState::ports`], [`AppState::state`] |
//! | cluster services | the state itself ([`ClusterServices`]: session manager, catalog, namespaces, integrations) | [`AppState::services`] |
//! | command bus | set once by the main window's mount ([`CommandBus`] with its `MutationGuard`) | [`AppState::command_bus`] |
//! | resource stores | set once by the main window's mount (`ResourceStores`: one `ResourceStore` per connected cluster) | [`AppState::resource_stores`] |
//! | log service | set once by the main window's mount (`LogService`: the log sessions of every cluster, bounded by `logs.buffer_lines`) | [`AppState::log_service`] |
//! | exec service | set once by the main window's mount (`ExecService`: shells, attaches and commands in pod containers, the last container chosen per pod) | [`AppState::exec_service`] |
//! | agent hooks | set once by the main window's mount (`AgentHooks`: the `@`-mention `ContextRegistry` with `@logs`, the `ToolRegistry` with `k8s.get_logs`, and the queue "Send to agent" fills) | [`AppState::agent_hooks`] |
//! | recent commands | the state itself (`RecentsStore`: the commands the palette ran lately, in memory until E11-S11 persists them) | [`AppState::recents`] |
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
//! trait objects, so nothing that takes an `AppState` knows SQLite, kube-rs or the keychain. The
//! `SecretStorePort` is wired when its adapter exists (`AppPorts::secrets` is `None` until then).
//!
//! The command bus is built when the main window is mounted (`crate::mount`), because the cluster
//! tab commands it routes need that window's tab controller; [`AppState::set_command_bus`] stores
//! it once and [`AppState::command_bus`] hands it out (the palette, MCP and extensions dispatch
//! there too, later).
//!
//! # Tests
//!
//! `AppState::test(cx)` (test builds and feature `test-support`) runs the real init order with testkit fakes and no OS
//! threads: an in-memory settings store, the deterministic runtime, no file watchers, a
//! `oxikube_testkit::FakeStatePort`. It is the same code path as the app
//! (`startup::init`), so a test that calls it exercises the order. `AppState::test_with(cx, &ports)`
//! does the same over a `oxikube_testkit::TestPorts` the test keeps, to script and assert on the fakes.

mod agent;
#[cfg(test)]
mod harness_tests;
mod ports;
mod services;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui::{App, Global};
use oxikube_app::logs::LogService;
use oxikube_app::{CommandBus, ExecService, MemoryRecents, RecentsStore, ResourceStores};
use oxikube_keymap::KeymapStore;
use oxikube_ports::StatePort;
use oxikube_runtime::RuntimeMode;
use oxikube_settings::SettingsStore;
use oxikube_theme::{ActiveTheme, ThemeRegistry, ThemeTokens};

pub use agent::AgentHooks;
pub use ports::{AppPorts, ClusterAdapters};
pub use services::ClusterServices;

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
    services: ClusterServices,
    bus: OnceLock<CommandBus>,
    stores: OnceLock<Arc<ResourceStores>>,
    logs: OnceLock<Arc<LogService>>,
    exec: OnceLock<Arc<ExecService>>,
    agent: OnceLock<AgentHooks>,
    recents: Arc<dyn RecentsStore>,
    data_dir: Option<PathBuf>,
}

struct GlobalAppState(Arc<AppState>);

impl Global for GlobalAppState {}

impl AppState {
    /// A state over `ports`. `data_dir` is where logs, crash reports and the state database live
    /// (`None` for tests and when the OS reports none). It is not installed yet: see
    /// [`AppState::install`].
    ///
    /// Builds the [`ClusterServices`] over the ports (cheap: nothing is read or spawned).
    pub fn new(ports: AppPorts, data_dir: Option<PathBuf>) -> Self {
        Self {
            services: ClusterServices::new(&ports),
            ports,
            bus: OnceLock::new(),
            stores: OnceLock::new(),
            logs: OnceLock::new(),
            exec: OnceLock::new(),
            agent: OnceLock::new(),
            recents: Arc::new(MemoryRecents::new()),
            data_dir,
        }
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

    /// The cluster services (sessions, catalog, namespaces, integrations).
    pub fn services(&self) -> &ClusterServices {
        &self.services
    }

    /// The command bus, once the main window has been mounted (`None` before).
    pub fn command_bus(&self) -> Option<&CommandBus> {
        self.bus.get()
    }

    /// Stores the command bus. The first one stays: `false` (and `bus` is dropped) when one was
    /// set already.
    pub fn set_command_bus(&self, bus: CommandBus) -> bool {
        self.bus.set(bus).is_ok()
    }

    /// The per-cluster resource stores behind every table, sidebar count and overview tile, once
    /// the main window has been mounted (`None` before). One per app: the views of every window
    /// share a cluster's feeds.
    pub fn resource_stores(&self) -> Option<&Arc<ResourceStores>> {
        self.stores.get()
    }

    /// Stores the resource stores. The first ones stay: `false` when some were set already.
    pub fn set_resource_stores(&self, stores: Arc<ResourceStores>) -> bool {
        self.stores.set(stores).is_ok()
    }

    /// The log service behind every log viewer, once the main window has been mounted (`None`
    /// before). One per app: the sessions of every cluster share the `logs.buffer_lines` bound.
    pub fn log_service(&self) -> Option<&Arc<LogService>> {
        self.logs.get()
    }

    /// Stores the log service. The first one stays: `false` when one was set already.
    pub fn set_log_service(&self, service: Arc<LogService>) -> bool {
        self.logs.set(service).is_ok()
    }

    /// The exec service behind `pod::Shell`, `pod::Attach` and the container picker, once the main
    /// window has been mounted (`None` before). One per app: the container chosen last in each pod
    /// is remembered across windows.
    pub fn exec_service(&self) -> Option<&Arc<ExecService>> {
        self.exec.get()
    }

    /// Stores the exec service. The first one stays: `false` when one was set already.
    pub fn set_exec_service(&self, service: Arc<ExecService>) -> bool {
        self.exec.set(service).is_ok()
    }

    /// The agent-facing registries (mention providers, tools) and the pending-context queue, once
    /// the main window has been mounted (`None` before). One per app.
    pub fn agent_hooks(&self) -> Option<&AgentHooks> {
        self.agent.get()
    }

    /// Stores the agent hooks. The first ones stay: `false` when some were set already.
    pub fn set_agent_hooks(&self, hooks: AgentHooks) -> bool {
        self.agent.set(hooks).is_ok()
    }

    /// The commands the palette ran lately, shared by every window.
    pub fn recents(&self) -> &Arc<dyn RecentsStore> {
        &self.recents
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

    /// The real init order over the fakes of `ports` (`oxikube_testkit::TestPorts`), so the test
    /// keeps its handles: script `ports.state`, then assert on `ports.state.recorded_calls()`.
    ///
    /// Must be the first `AppState` call of the app: ports cannot be swapped under an installed
    /// state, so it panics when one is already installed.
    #[track_caller]
    pub fn test_with(cx: &mut App, ports: &oxikube_testkit::TestPorts) -> Arc<AppState> {
        assert!(
            Self::try_global(cx).is_none(),
            "AppState::test_with must run before any other AppState::test call in the app"
        );
        crate::startup::init(cx, crate::startup::StartupEnv::test_with(ports))
            .expect("the test init order runs");
        Self::global(cx)
    }
}
