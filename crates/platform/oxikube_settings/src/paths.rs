//! Where the user's settings live, and first-run creation of `settings.json`.
//!
//! The config dir is `$OXIKUBE_CONFIG_DIR` when set, else `$XDG_CONFIG_HOME/oxikube` or
//! `~/.config/oxikube` on macOS and Linux (like Zed: a dotfile-friendly place for hand-edited
//! JSON), and `%APPDATA%\Oxikube` on Windows.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use oxikube_domain::{OxiError, OxiResult};

/// Environment variable that overrides the config dir (tests, portable installs).
pub const CONFIG_DIR_ENV: &str = "OXIKUBE_CONFIG_DIR";
/// File name of the user settings inside the config dir.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// The config dir, from the environment. `None` when no home directory is known.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        dirs::home_dir(),
    )
}

/// [`config_dir`] with its inputs explicit (pure, for tests).
pub fn config_dir_from(
    override_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = override_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        return Some(dir);
    }
    if cfg!(windows) {
        return dirs::config_dir().map(|dir| dir.join("Oxikube"));
    }
    if let Some(dir) = xdg_config_home.filter(|dir| dir.is_absolute()) {
        return Some(dir.join("oxikube"));
    }
    home.map(|home| home.join(".config").join("oxikube"))
}

/// The user settings file inside `config_dir`.
pub fn user_settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

/// Read the user settings at `path`, creating the dir and a commented template on first run.
///
/// Returns the file's text (the template when it was just created).
pub fn load_or_create_user_settings(path: &Path) -> OxiResult<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let template = oxikube_assets::initial_user_settings_content();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|err| io_error("create", dir, err))?;
            }
            std::fs::write(path, template).map_err(|err| io_error("write", path, err))?;
            Ok(template.to_owned())
        }
        Err(err) => Err(io_error("read", path, err)),
    }
}

/// An `Internal` error for a failed file operation (paths are not secret).
pub(crate) fn io_error(action: &str, path: &Path, err: std::io::Error) -> OxiError {
    OxiError::internal(format!("could not {action} {}: {err}", path.display())).with_source(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn override_beats_xdg_beats_home() {
        let home = Some(PathBuf::from("/home/me"));
        let xdg = Some(PathBuf::from("/xdg"));
        assert_eq!(
            config_dir_from(Some("/custom".into()), xdg.clone(), home.clone()),
            Some(PathBuf::from("/custom"))
        );
        assert_eq!(
            config_dir_from(None, xdg, home.clone()),
            Some(PathBuf::from("/xdg/oxikube"))
        );
        assert_eq!(
            config_dir_from(Some("".into()), Some("relative".into()), home),
            Some(PathBuf::from("/home/me/.config/oxikube"))
        );
        assert_eq!(config_dir_from(None, None, None), None);
    }

    #[test]
    fn first_run_writes_the_template_and_later_runs_read_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = user_settings_path(&dir.path().join("nested"));
        let text = load_or_create_user_settings(&path).unwrap();
        assert_eq!(text, oxikube_assets::initial_user_settings_content());
        assert!(crate::jsonc::parse_jsonc_object(&text).unwrap().is_empty());

        std::fs::write(&path, "{\"a\": 1}").unwrap();
        assert_eq!(load_or_create_user_settings(&path).unwrap(), "{\"a\": 1}");
    }
}
