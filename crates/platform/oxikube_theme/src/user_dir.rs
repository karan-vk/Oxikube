//! The user's `themes/` directory: scanning it into theme families.
//!
//! Independent of GPUI. Called synchronously by tests and by the watcher thread (never by the
//! UI thread in the app), because it reads and parses files.

use crate::import::{ThemeFamily, import_family};
use crate::registry::ThemeFileProblem;
use std::path::{Path, PathBuf};

/// Name of the themes directory inside the config dir.
const THEMES_DIR_NAME: &str = "themes";

/// Theme files larger than this are skipped (a real family is tens of kilobytes).
const MAX_THEME_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The themes directory inside `config_dir`.
pub fn themes_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(THEMES_DIR_NAME)
}

/// The result of scanning the themes directory once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserThemes {
    /// Families that parsed, with the file each came from, in file-name order.
    pub families: Vec<(PathBuf, ThemeFamily)>,
    /// Files that could not be used, and why.
    pub problems: Vec<ThemeFileProblem>,
}

/// Reads every `*.json` file directly inside `dir`.
///
/// A missing directory is an empty result (most users have none). Unreadable or invalid files
/// become [`ThemeFileProblem`]s; a file whose themes import with some bad values still
/// contributes its themes and reports the first bad value. Files are read in name order so
/// duplicate theme names resolve the same way every time.
pub fn scan_dir(dir: &Path) -> UserThemes {
    let mut scan = UserThemes::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return scan;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .filter(|path| !is_hidden(path) && path.is_file())
        .collect();
    files.sort();

    for path in files {
        match read_family(&path) {
            Ok((family, problem)) => {
                if let Some(message) = problem {
                    scan.problems.push(ThemeFileProblem {
                        path: path.clone(),
                        message,
                    });
                }
                scan.families.push((path, family));
            }
            Err(message) => scan.problems.push(ThemeFileProblem { path, message }),
        }
    }
    scan
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') || name.starts_with("~"))
}

/// The family in `path` and, when some value was invalid, the first complaint.
fn read_family(path: &Path) -> Result<(ThemeFamily, Option<String>), String> {
    let meta = std::fs::metadata(path).map_err(|err| format!("cannot read: {err}"))?;
    if meta.len() > MAX_THEME_FILE_BYTES {
        return Err(format!(
            "file is {} bytes; theme files are limited to 8 MiB",
            meta.len()
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|err| format!("cannot read: {err}"))?;
    let imported = import_family(&text).map_err(|err| err.to_string())?;
    let problem = imported.report.diagnostics.first().map(ToString::to_string);
    Ok((imported.family, problem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::AYU;

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(scan_dir(&dir.path().join("nope")), UserThemes::default());
    }

    #[test]
    fn scans_json_files_in_name_order_and_reports_bad_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b-ayu.json"), AYU).unwrap();
        std::fs::write(
            dir.path().join("a-mine.json"),
            r##"{ "name": "Mine", "themes": [{ "name": "Mine", "appearance": "dark", "style": { "text": "bad" } }] }"##,
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ nope").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.path().join(".hidden.json"), AYU).unwrap();
        std::fs::create_dir(dir.path().join("sub.json")).unwrap();

        let scan = scan_dir(dir.path());
        let names: Vec<_> = scan
            .families
            .iter()
            .map(|(_, family)| family.name.as_str())
            .collect();
        assert_eq!(names, ["Mine", "Ayu"]);
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
        assert!(scan.problems[0].path.ends_with("a-mine.json"));
        assert!(scan.problems[0].message.contains("`text` is not a colour"));
        assert!(scan.problems[1].path.ends_with("broken.json"));
    }
}
