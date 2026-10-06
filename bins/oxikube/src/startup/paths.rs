//! Where the app keeps its own files.

use std::path::{Path, PathBuf};

/// Overrides the data directory (logs, crash reports, the state database).
pub const DATA_DIR_ENV: &str = "OXIKUBE_DATA_DIR";

/// `$OXIKUBE_DATA_DIR`, else `<OS data dir>/oxikube` (`~/Library/Application Support/oxikube`,
/// `~/.local/share/oxikube`), or `None` when the OS reports none.
///
/// Settings, keymap and themes are not here: they live in the config directory
/// (`oxikube_settings::paths::config_dir`), which a user edits and syncs; this one is the app's.
pub fn data_dir() -> Option<PathBuf> {
    match std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => dirs::data_dir().map(|d| d.join("oxikube")),
    }
}

/// `<data dir>/logs`.
pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// `<data dir>/crashes`.
pub fn crash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("crashes")
}

/// `<data dir>/state.db`.
pub fn state_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state.db")
}

/// Where pasted kubeconfigs are stored: `<config dir>/kubeconfigs` (ADR 0015). For the embedded
/// defaults (tests, no config dir) a directory under the system temp dir, which nothing writes to
/// unless the user pastes a kubeconfig in such a run.
pub fn kubeconfigs_dir(config: &super::ConfigSource) -> PathBuf {
    let dir = match config {
        super::ConfigSource::UserDir => oxikube_settings::paths::config_dir(),
        super::ConfigSource::Dir(dir) => Some(dir.clone()),
        super::ConfigSource::Memory => None,
    };
    dir.unwrap_or_else(|| std::env::temp_dir().join("oxikube"))
        .join("kubeconfigs")
}
