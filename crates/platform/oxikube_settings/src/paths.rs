//! Where the user's settings live, first-run creation of `settings.json`, and its atomic
//! rewrite.
//!
//! The config dir is `$OXIKUBE_CONFIG_DIR` when set, else `$XDG_CONFIG_HOME/oxikube` or
//! `~/.config/oxikube` on macOS and Linux (like Zed: a dotfile-friendly place for hand-edited
//! JSON), and `%APPDATA%\Oxikube` on Windows.

use std::io::{ErrorKind, Write as _};
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

/// Replace the settings file at `path` with `text` atomically.
///
/// The text goes to a temporary file in the same directory, is flushed to disk, then renamed
/// over the target, so a crash, a full disk or a failed write leaves the old file intact and
/// the hot-reload watcher never sees a truncated file. A symlinked `settings.json` (dotfile
/// managers) keeps its link: the link's target is replaced. The target's permissions carry
/// over to the new file.
pub fn write_atomically(path: &Path, text: &str) -> OxiResult<()> {
    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(err) if err.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(err) => return Err(io_error("resolve", path, err)),
    };
    let file_name = target
        .file_name()
        .ok_or_else(|| OxiError::internal(format!("{} has no file name", path.display())))?;
    // Per process, so two running instances never share a temporary file; edits within one
    // process are serialised by the caller.
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = target.with_file_name(tmp_name);

    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        if let Ok(metadata) = std::fs::metadata(&target) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    written.map_err(|err| {
        // Best effort: the temporary file may not exist, and the original is untouched.
        let _ = std::fs::remove_file(&tmp);
        io_error("write", path, err)
    })
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

    /// The new text replaces the file by rename: a reader holding the old file still sees
    /// the complete old text (an in-place truncate-and-write would have changed it), and no
    /// temporary file is left behind.
    #[test]
    #[cfg(unix)]
    fn writes_replace_the_file_by_rename() {
        use std::io::Read as _;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE_NAME);
        std::fs::write(&path, "{\"old\": 1}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut old = std::fs::File::open(&path).unwrap();

        write_atomically(&path, "{\"new\": 2}").unwrap();

        let mut old_text = String::new();
        old.read_to_string(&mut old_text).unwrap();
        assert_eq!(old_text, "{\"old\": 1}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"new\": 2}");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [SETTINGS_FILE_NAME]);
    }

    /// A failed write leaves the old file as it was.
    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE_NAME);
        std::fs::write(&path, "{\"old\": 1}").unwrap();
        // The temporary file's name is taken by a directory, so creating it fails.
        let tmp = dir
            .path()
            .join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", std::process::id()));
        std::fs::create_dir(&tmp).unwrap();

        assert!(write_atomically(&path, "{\"new\": 2}").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"old\": 1}");
    }

    /// A symlinked settings file (dotfile managers) stays a link; its target gets the text.
    #[test]
    #[cfg(unix)]
    fn writes_through_a_symlink_keep_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("dotfiles");
        std::fs::create_dir(&real_dir).unwrap();
        let real = real_dir.join("oxikube.json");
        std::fs::write(&real, "{}").unwrap();
        let link = dir.path().join(SETTINGS_FILE_NAME);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write_atomically(&link, "{\"a\": 1}").unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"a\": 1}");
    }
}
