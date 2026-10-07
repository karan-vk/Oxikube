//! Where the app keeps its own files.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Overrides the data directory (logs, crash reports, the state database).
pub const DATA_DIR_ENV: &str = "OXIKUBE_DATA_DIR";

/// `$OXIKUBE_DATA_DIR`, else `<OS data dir>/oxikube` (`~/Library/Application Support/oxikube`,
/// `~/.local/share/oxikube`), or `None` when the OS reports none.
///
/// Settings, keymap and themes are not here: they live in the config directory
/// (`oxikube_settings::paths::config_dir`), which a user edits and syncs; this one is the app's.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(std::env::var_os(DATA_DIR_ENV))
}

/// [`data_dir`] with the `OXIKUBE_DATA_DIR` value passed in (`None` or empty = unset), so tests do
/// not mutate the process environment.
fn data_dir_from(env: Option<OsString>) -> Option<PathBuf> {
    match env.filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => dirs::data_dir().map(|d| d.join("oxikube")),
    }
}

/// `<data dir>/perf`: where `--perf` writes its JSONL unless `--perf-dir` says otherwise.
pub fn perf_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("perf")
}

/// The default `--perf` directory: [`perf_dir`] of [`data_dir`], so `OXIKUBE_DATA_DIR` isolates
/// perf logs together with the logs, crash reports and state database. `None` when the OS
/// reports no data directory.
pub fn default_perf_dir() -> Option<PathBuf> {
    data_dir().map(|d| perf_dir(&d))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_moves_the_data_dir_and_the_perf_dir() {
        let data = data_dir_from(Some("/tmp/iso".into())).unwrap();
        assert_eq!(data, PathBuf::from("/tmp/iso"));
        assert_eq!(perf_dir(&data), PathBuf::from("/tmp/iso/perf"));
    }

    #[test]
    fn unset_or_empty_env_falls_back_to_the_os_data_dir() {
        for env in [None, Some(OsString::new())] {
            if let Some(data) = data_dir_from(env) {
                assert!(data.ends_with("oxikube"));
                assert!(perf_dir(&data).ends_with("oxikube/perf"));
            }
        }
    }
}
