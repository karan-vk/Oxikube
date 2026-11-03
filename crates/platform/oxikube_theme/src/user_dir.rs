//! The user's `themes/` directory: scanning it into theme families.
//!
//! Independent of GPUI. Called synchronously by tests and by the watcher thread (never by the
//! UI thread in the app), because it reads and parses files.

use crate::import::{ThemeFamily, import_family};
use crate::registry::ThemeFileProblem;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

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
    scan_dir_cached(dir, &mut ScanCache::default())
}

/// What reading one file produced: its family and first complaint, or why it failed.
type FileOutcome = Result<(ThemeFamily, Option<String>), String>;

/// One remembered file: its stamp, whether the stamp can be trusted, and what it produced.
#[derive(Debug)]
struct CachedFile {
    modified: Option<SystemTime>,
    len: u64,
    /// The file was last modified well before it was read. A file touched within
    /// [`RACY_WINDOW`] of the read could change again without moving its timestamp (coarse
    /// filesystem clocks), so it is read again next time.
    stable: bool,
    outcome: FileOutcome,
}

/// Timestamps closer than this to a read are not trusted (as in git's "racily clean" rule).
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Remembers what each file produced, so a rescan only re-imports files that changed.
///
/// A file counts as unchanged when its modification time and length are. Entries of files that
/// are gone are dropped on the next scan.
#[derive(Debug, Default)]
pub struct ScanCache {
    files: BTreeMap<PathBuf, CachedFile>,
    /// How many files the last scan had to read and parse (the rest came from the cache).
    pub imported: usize,
}

/// Like [`scan_dir`], re-importing only files that differ from what `cache` remembers.
pub fn scan_dir_cached(dir: &Path, cache: &mut ScanCache) -> UserThemes {
    cache.imported = 0;
    let mut kept = BTreeMap::new();
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
        let stamp = std::fs::metadata(&path)
            .ok()
            .map(|meta| (meta.modified().ok(), meta.len()));
        let outcome = match (stamp, cache.files.remove(&path)) {
            (Some((modified, len)), Some(cached))
                if cached.stable
                    && modified.is_some()
                    && cached.modified == modified
                    && cached.len == len =>
            {
                cached.outcome
            }
            _ => {
                cache.imported += 1;
                read_family(&path)
            }
        };
        if let Some((modified, len)) = stamp {
            let stable = modified
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .is_some_and(|age| age > RACY_WINDOW);
            kept.insert(
                path.clone(),
                CachedFile {
                    modified,
                    len,
                    stable,
                    outcome: outcome.clone(),
                },
            );
        }
        match outcome {
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
    cache.files = kept;
    scan
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') || name.starts_with("~"))
}

/// The family in `path` and, when some value was invalid, the first complaint.
fn read_family(path: &Path) -> FileOutcome {
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

    /// Makes a file look old enough for its timestamp to be trusted.
    fn backdate(path: &Path) {
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    #[test]
    fn a_file_changed_just_now_is_read_again_even_if_the_stamp_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), AYU).unwrap();
        let mut cache = ScanCache::default();
        scan_dir_cached(dir.path(), &mut cache);
        scan_dir_cached(dir.path(), &mut cache);
        assert_eq!(cache.imported, 1, "a fresh file is not trusted yet");
    }

    #[test]
    fn a_cached_rescan_only_imports_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), AYU).unwrap();
        std::fs::write(dir.path().join("b.json"), "{ nope").unwrap();
        for name in ["a.json", "b.json"] {
            backdate(&dir.path().join(name));
        }
        let mut cache = ScanCache::default();
        let first = scan_dir_cached(dir.path(), &mut cache);
        assert_eq!(cache.imported, 2);
        let second = scan_dir_cached(dir.path(), &mut cache);
        assert_eq!(cache.imported, 0, "nothing changed");
        assert_eq!(first, second);

        std::fs::write(dir.path().join("b.json"), "{ nope still   }").unwrap();
        backdate(&dir.path().join("b.json"));
        scan_dir_cached(dir.path(), &mut cache);
        assert_eq!(cache.imported, 1, "only the edited file is read again");

        std::fs::remove_file(dir.path().join("a.json")).unwrap();
        let after_delete = scan_dir_cached(dir.path(), &mut cache);
        assert!(after_delete.families.is_empty());
        assert_eq!(cache.files.len(), 1, "a deleted file leaves the cache");
    }

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
