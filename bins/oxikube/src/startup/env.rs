//! What [`super::init`] is told about its surroundings: the same code path serves the app and
//! the tests, only the environment differs.

use std::path::PathBuf;

use oxikube_logging::LogHandle;

use super::stage::StageTiming;
use crate::app_state::AppPorts;

/// Where settings, keymap and themes come from.
#[derive(Debug, Clone)]
pub enum ConfigSource {
    /// The user's config directory (`$OXIKUBE_CONFIG_DIR` or `~/.config/oxikube`), created on
    /// first run, with hot reload (file watcher threads).
    UserDir,
    /// `<dir>/settings.json`, `<dir>/keymap.json`, `<dir>/themes/`, read once, no watchers.
    Dir(PathBuf),
    /// The embedded defaults only; nothing is read or written.
    Memory,
}

/// Which tokio backend `spawn_kube` runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChoice {
    /// A tokio runtime of our own (the app).
    Tokio,
    /// GPUI's background executor, no OS threads (`#[gpui::test]`).
    Deterministic,
}

/// How the ports bundle is built.
#[derive(Clone)]
pub enum PortsChoice {
    /// The SQLite adapter at this path, opened in the background (`:memory:` for a throwaway
    /// database), and no secret store yet.
    Sqlite(PathBuf),
    /// A bundle built by the caller (testkit fakes).
    Provided(AppPorts),
}

/// Inputs of [`super::init`].
#[derive(Clone)]
pub struct StartupEnv {
    /// Where settings, keymap and themes come from.
    pub config: ConfigSource,
    /// The tokio backend.
    pub runtime: RuntimeChoice,
    /// The ports bundle.
    pub ports: PortsChoice,
    /// The data directory ([`crate::app_state::AppState::data_dir`]).
    pub data_dir: Option<PathBuf>,
    /// The live log filter handle, when logging was set up; the `log.filter` setting follows it.
    pub log: Option<LogHandle>,
    /// Costs of the stages that ran before GPUI (logging, assets).
    pub earlier: Vec<StageTiming>,
}

impl StartupEnv {
    /// The app: the user's config directory with hot reload, a tokio runtime of our own, the
    /// SQLite state database in `data_dir` (a throwaway in-memory one when the OS reports no data
    /// directory).
    pub fn app(boot: super::Boot) -> Self {
        let state_path = match &boot.data_dir {
            Some(dir) => super::paths::state_db_path(dir),
            None => {
                tracing::warn!("no data directory: the state database is not persisted");
                PathBuf::from(":memory:")
            }
        };
        Self {
            config: ConfigSource::UserDir,
            runtime: RuntimeChoice::Tokio,
            ports: PortsChoice::Sqlite(state_path),
            data_dir: boot.data_dir,
            log: boot.log,
            earlier: boot.timings,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl StartupEnv {
    /// Test fakes, no disk, no OS threads: the embedded defaults, the deterministic runtime and a
    /// `FakeStatePort` / `FakeSecretStorePort` bundle.
    pub fn test() -> Self {
        use std::sync::Arc;
        let ports = AppPorts::new(Arc::new(oxikube_testkit::FakeStatePort::new()))
            .with_secrets(Arc::new(oxikube_testkit::FakeSecretStorePort::new()));
        Self {
            config: ConfigSource::Memory,
            runtime: RuntimeChoice::Deterministic,
            ports: PortsChoice::Provided(ports),
            data_dir: None,
            log: None,
            earlier: Vec::new(),
        }
    }
}
