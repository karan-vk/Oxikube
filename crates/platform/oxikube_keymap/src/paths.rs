//! Where the user's `keymap.json` lives: next to `settings.json`.

use std::path::{Path, PathBuf};

use oxikube_domain::{OxiError, OxiResult};

/// File name of the user keymap inside the config dir.
pub const KEYMAP_FILE_NAME: &str = "keymap.json";

/// The user keymap inside `config_dir` (see `oxikube_settings::paths::config_dir`).
pub fn user_keymap_path(config_dir: &Path) -> PathBuf {
    config_dir.join(KEYMAP_FILE_NAME)
}

/// The text of the keymap at `path`; a missing file is an empty keymap, not an error.
pub(crate) fn read_or_empty(path: &Path) -> OxiResult<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => {
            Err(OxiError::internal(format!("could not read {}", path.display())).with_source(err))
        }
    }
}

/// Create `keymap.json` from the commented template when it does not exist, for "open keymap"
/// (E21). Returns the path. Never overwrites an existing file.
pub fn ensure_user_keymap(config_dir: &Path) -> OxiResult<PathBuf> {
    let path = user_keymap_path(config_dir);
    if !path.exists() {
        std::fs::create_dir_all(config_dir).map_err(|err| {
            OxiError::internal(format!("could not create {}", config_dir.display()))
                .with_source(err)
        })?;
        oxikube_settings::paths::write_atomically(
            &path,
            oxikube_assets::initial_user_keymap_content(),
        )?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reads_empty_and_the_template_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = user_keymap_path(dir.path());
        assert_eq!(read_or_empty(&path).unwrap(), "");

        assert_eq!(ensure_user_keymap(dir.path()).unwrap(), path);
        let template = std::fs::read_to_string(&path).unwrap();
        assert_eq!(template, oxikube_assets::initial_user_keymap_content());

        std::fs::write(&path, "[]").unwrap();
        ensure_user_keymap(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
    }
}
