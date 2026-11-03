//! A temporary config directory with a seeded `settings.json`, for the tests of the settings
//! page and of anything that installs the settings store over a directory (`init_with_dir`, no
//! watcher). One helper here instead of a copy per crate.

use std::path::{Path, PathBuf};

/// The name of the user's settings file inside a config directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// A temporary config directory, removed when dropped.
#[derive(Debug)]
pub struct TempConfigDir {
    dir: tempfile::TempDir,
}

impl TempConfigDir {
    /// A directory whose `settings.json` holds `settings_text`.
    ///
    /// # Panics
    ///
    /// When the temporary directory cannot be created or written: the test machine is broken.
    #[must_use]
    pub fn with_settings(settings_text: &str) -> Self {
        let dir = tempfile::tempdir().expect("a temporary config directory");
        std::fs::write(dir.path().join(SETTINGS_FILE), settings_text)
            .expect("the temporary config directory is writable");
        Self { dir }
    }

    /// The directory, to hand to `init_with_dir`.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The path of `settings.json`.
    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.dir.path().join(SETTINGS_FILE)
    }

    /// The current text of `settings.json`.
    ///
    /// # Panics
    ///
    /// When the file is missing or unreadable.
    #[must_use]
    pub fn settings_text(&self) -> String {
        std::fs::read_to_string(self.settings_path()).expect("settings.json exists")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_and_reads_back_the_file() {
        let dir = TempConfigDir::with_settings("{ \"a\": 1 }");
        assert_eq!(dir.settings_text(), "{ \"a\": 1 }");
        assert!(dir.settings_path().starts_with(dir.path()));
    }
}
