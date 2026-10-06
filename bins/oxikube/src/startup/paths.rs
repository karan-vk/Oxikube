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
