//! The user's `icon_themes/` directory: every `*.json` family file directly in it, with icon
//! paths relative to the directory itself (so a theme keeps its SVGs in a subfolder:
//! `icon_themes/material.json` naming `./material/pod.svg`).
//!
//! Independent of GPUI and blocking: run by the watcher thread or a test, never the UI thread.

use super::read::{IconThemeSource, read_icon_theme_file};
use crate::registry::ThemeFileProblem;
use std::path::{Path, PathBuf};

/// Name of the icon-themes directory inside the config dir.
const ICON_THEMES_DIR_NAME: &str = "icon_themes";

/// The icon-themes directory inside `config_dir`.
pub fn icon_themes_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(ICON_THEMES_DIR_NAME)
}

/// The result of scanning the icon-themes directory once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserIconThemes {
    /// Families that parsed, with the file each came from, in file-name order.
    pub families: Vec<(PathBuf, IconThemeSource)>,
    /// Files that could not be used, or icons a usable file had to drop.
    pub problems: Vec<ThemeFileProblem>,
}

/// Reads every family file in `dir` and the SVGs each names. A missing directory is an empty
/// result.
pub fn scan_icon_themes_dir(dir: &Path) -> UserIconThemes {
    let mut scan = UserIconThemes::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return scan;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let visible = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('.') && !name.starts_with('~'));
            let json = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
            visible && json && path.is_file()
        })
        .collect();
    files.sort();
    for path in files {
        match read_icon_theme_file(&path, dir) {
            Ok(source) => {
                let dropped = source.problems.iter().map(|message| ThemeFileProblem {
                    path: path.clone(),
                    message: message.clone(),
                });
                scan.problems.extend(dropped);
                scan.families.push((path, source));
            }
            Err(err) => scan.problems.push(ThemeFileProblem {
                path,
                message: err.to_string(),
            }),
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watcher::{DEFAULT_DEBOUNCE, ThemeDirWatcher};
    use notify::RecursiveMode;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const THEME: &str = r#"{ "name": "Mine", "themes": [{ "name": "Mine", "appearance": "dark",
        "file_icons": { "kind:Pod": { "path": "./mine/pod.svg" }, "kind:Node": { "path": "../x.svg" } } }] }"#;

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            scan_icon_themes_dir(&dir.path().join("nope")),
            UserIconThemes::default()
        );
    }

    #[test]
    fn scans_family_files_and_reports_bad_files_and_dropped_icons() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("mine")).unwrap();
        std::fs::write(dir.path().join("mine/pod.svg"), "<svg/>").unwrap();
        std::fs::write(dir.path().join("mine.json"), THEME).unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ nope").unwrap();
        std::fs::write(dir.path().join(".hidden.json"), THEME).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "no").unwrap();

        let scan = scan_icon_themes_dir(dir.path());
        assert_eq!(scan.families.len(), 1);
        let (path, source) = &scan.families[0];
        assert!(path.ends_with("mine.json"));
        assert_eq!(source.files.len(), 1);
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
        assert!(scan.problems[0].path.ends_with("broken.json"));
        assert!(scan.problems[1].message.contains("leaves"));
    }

    /// Real directory, real watcher thread, no GPUI: a family dropped into a subfolder layout
    /// shows up, and editing one of its SVGs (in the subfolder) triggers a new scan.
    #[test]
    fn the_watcher_reports_new_families_and_edited_icons() {
        let dir = tempfile::tempdir().unwrap();
        let root = icon_themes_dir(dir.path());
        let (tx, rx) = mpsc::channel();
        let _watcher = ThemeDirWatcher::spawn_scanning(
            root.clone(),
            DEFAULT_DEBOUNCE,
            RecursiveMode::Recursive,
            scan_icon_themes_dir,
            move |scan| tx.send(scan).is_ok(),
        )
        .unwrap();
        let wait = |pred: &dyn Fn(&UserIconThemes) -> bool, timeout: Duration| {
            let deadline = Instant::now() + timeout;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(scan) if pred(&scan) => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
            false
        };
        assert!(wait(
            &|scan| scan.families.is_empty(),
            Duration::from_secs(10)
        ));

        std::fs::create_dir_all(root.join("mine")).unwrap();
        let svg = |text: &str| std::fs::write(root.join("mine/pod.svg"), text).unwrap();
        svg("<svg>1</svg>");
        // The platform watch starts asynchronously (FSEvents): rewrite until it lands.
        let landed = (0..20).any(|_| {
            std::fs::write(root.join("mine.json"), THEME).unwrap();
            wait(&|scan| scan.families.len() == 1, Duration::from_millis(500))
        });
        assert!(landed, "no scan with the new family within 10 s");

        let edited = (0..20).any(|round| {
            svg(&format!("<svg>edit {round}</svg>"));
            wait(
                &|scan| {
                    scan.families.first().is_some_and(|(_, source)| {
                        source
                            .files
                            .values()
                            .any(|bytes| bytes.starts_with(b"<svg>edit"))
                    })
                },
                Duration::from_millis(500),
            )
        });
        assert!(edited, "an edited SVG in a subfolder is picked up");
    }
}
