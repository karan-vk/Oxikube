//! Splitting the `KUBECONFIG` value into file paths.
//!
//! Pure: it takes the value (and the [`Platform`]) as arguments and never reads the process
//! environment, so tests need no `set_var` and can exercise both platforms from one machine.
//!
//! # Separator rules
//!
//! kubectl splits `KUBECONFIG` with Go's `filepath.SplitList`: the host's list separator only
//! (`:` on Unix, `;` on Windows), skips empty entries, and does **not** expand `~` (the shell
//! does that, and not inside quotes). Oxikube accepts both separators on both platforms, since
//! the value is often copied between machines, without breaking real paths:
//!
//! * **Unix**: `:` always separates. `;` separates only when it is directly followed by `/`
//!   (the next entry is an absolute path) or ends the value. A `;` inside a name such as
//!   `/home/me/a;b/config` is left alone.
//! * **Windows**: `;` always separates. `:` separates unless it is a drive colon, meaning the
//!   current entry so far is a single ASCII letter (`C:\Users\me\.kube\config`). Double quotes
//!   group (`"C:\a;b";D:\c`) and are removed, as `filepath.SplitList` does on Windows.
//!
//! Empty entries (leading, trailing and repeated separators) are dropped. No `~` or environment
//! variable expansion is done, matching kubectl.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Which `KUBECONFIG` separator rules to apply. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// `:` separates; `;` only before an absolute path or at the end.
    Unix,
    /// `;` separates; `:` unless it is a drive colon; double quotes group.
    Windows,
}

impl Platform {
    /// The platform this binary runs on.
    pub const fn host() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::host()
    }
}

/// Split a `KUBECONFIG` value into its paths, in order, using `platform`'s rules.
///
/// Empty entries are dropped. Duplicates are kept here; the loader de-duplicates by canonical
/// path. See the module docs for the rules.
pub fn split_kubeconfig(value: &str, platform: Platform) -> Vec<PathBuf> {
    let entries = match platform {
        Platform::Unix => split_unix(value),
        Platform::Windows => split_windows(value),
    };
    entries
        .into_iter()
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn split_unix(value: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let separator = match c {
            ':' => true,
            ';' => matches!(chars.peek(), None | Some('/')),
            _ => false,
        };
        if separator {
            entries.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    entries.push(current);
    entries
}

fn split_windows(value: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in value.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => entries.push(std::mem::take(&mut current)),
            ':' if !in_quotes && !is_single_letter(&current) => {
                entries.push(std::mem::take(&mut current))
            }
            _ => current.push(c),
        }
    }
    entries.push(current);
    entries
}

/// True when `entry` is exactly one ASCII letter: the `C` of a drive prefix `C:`.
fn is_single_letter(entry: &str) -> bool {
    let mut chars = entry.chars();
    matches!((chars.next(), chars.next()), (Some(letter), None) if letter.is_ascii_alphabetic())
}

/// [`split_kubeconfig`] for a value that may not be UTF-8.
///
/// A UTF-8 value uses `platform`'s rules. A non-UTF-8 value on the host platform falls back to
/// [`std::env::split_paths`] so no path is lost; on another platform it is split lossily.
pub fn split_kubeconfig_os(value: &OsStr, platform: Platform) -> Vec<PathBuf> {
    match value.to_str() {
        Some(text) => split_kubeconfig(text, platform),
        None if platform == Platform::host() => std::env::split_paths(value)
            .filter(|path| !path.as_os_str().is_empty())
            .collect(),
        None => split_kubeconfig(&value.to_string_lossy(), platform),
    }
}

/// Split a `KUBECONFIG` value into its paths, in order, using the host platform's rules.
///
/// Same as [`split_kubeconfig_os`] with [`Platform::host`].
pub fn split_kubeconfig_paths(value: &OsStr) -> Vec<PathBuf> {
    split_kubeconfig_os(value, Platform::host())
}

/// Resolve the kubeconfig paths to load from an explicit `KUBECONFIG` value and the default path.
///
/// An unset or empty (all-separators) `KUBECONFIG` falls back to `default_path`
/// (`~/.kube/config`), as kubectl does. `default_path` is `None` when the home directory is
/// unknown, which yields no paths.
pub fn resolve_kubeconfig_paths(
    kubeconfig_env: Option<&OsStr>,
    default_path: Option<&Path>,
) -> Vec<PathBuf> {
    let from_env = kubeconfig_env
        .map(split_kubeconfig_paths)
        .unwrap_or_default();
    if from_env.is_empty() {
        default_path.map(Path::to_path_buf).into_iter().collect()
    } else {
        from_env
    }
}

/// The default kubeconfig path under a home directory: `<home>/.kube/config`.
pub fn default_kubeconfig_path(home: &Path) -> PathBuf {
    home.join(".kube").join("config")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn join(parts: &[&str]) -> OsString {
        std::env::join_paths(parts).expect("test paths contain no separator")
    }

    #[test]
    fn splits_in_order() {
        let value = join(&["/a/config", "/b/config", "/c/config"]);
        let paths = split_kubeconfig_paths(&value);
        assert_eq!(
            paths,
            [
                PathBuf::from("/a/config"),
                PathBuf::from("/b/config"),
                PathBuf::from("/c/config")
            ]
        );
    }

    #[test]
    fn drops_empty_entries() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let value = OsString::from(format!("{sep}/a{sep}{sep}/b{sep}"));
        assert_eq!(
            split_kubeconfig_paths(&value),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn empty_value_yields_nothing() {
        assert!(split_kubeconfig_paths(OsStr::new("")).is_empty());
    }

    #[test]
    fn keeps_duplicates() {
        let value = join(&["/a", "/a"]);
        assert_eq!(split_kubeconfig_paths(&value).len(), 2);
    }

    #[test]
    fn unset_env_uses_default() {
        let default = Path::new("/home/u/.kube/config");
        assert_eq!(
            resolve_kubeconfig_paths(None, Some(default)),
            [default.to_path_buf()]
        );
    }

    #[test]
    fn empty_env_uses_default() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let default = Path::new("/home/u/.kube/config");
        for value in ["", sep, &format!("{sep}{sep}")] {
            assert_eq!(
                resolve_kubeconfig_paths(Some(OsStr::new(value)), Some(default)),
                [default.to_path_buf()],
                "value {value:?}"
            );
        }
    }

    #[test]
    fn env_wins_over_default() {
        let value = join(&["/x"]);
        let default = Path::new("/home/u/.kube/config");
        assert_eq!(
            resolve_kubeconfig_paths(Some(&value), Some(default)),
            [PathBuf::from("/x")]
        );
    }

    #[test]
    fn no_env_no_default_yields_nothing() {
        assert!(resolve_kubeconfig_paths(None, None).is_empty());
    }

    #[test]
    fn default_path_is_under_dot_kube() {
        assert_eq!(
            default_kubeconfig_path(Path::new("/home/u")),
            Path::new("/home/u").join(".kube").join("config")
        );
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    /// `(value, expected)` rows, run for the platform under test.
    fn check(platform: Platform, table: &[(&str, &[&str])]) {
        for (value, expected) in table {
            assert_eq!(
                split_kubeconfig(value, platform),
                paths(expected),
                "{platform:?} {value:?}"
            );
        }
    }

    #[test]
    fn unix_table() {
        check(
            Platform::Unix,
            &[
                ("/a/config", &["/a/config"]),
                ("/a:/b:/c", &["/a", "/b", "/c"]),
                ("", &[]),
                (":", &[]),
                ("::", &[]),
                (":/a::/b:", &["/a", "/b"]),
                ("/a/config:", &["/a/config"]),
                // `;` before an absolute path or at the end separates (a Windows-style list).
                ("/a;/b", &["/a", "/b"]),
                ("/a;/b;", &["/a", "/b"]),
                ("/a:/b;/c", &["/a", "/b", "/c"]),
                // `;` inside a name is part of the path.
                ("/home/me/a;b/config", &["/home/me/a;b/config"]),
                ("/a;b:/c", &["/a;b", "/c"]),
                // Windows drive paths mean nothing on Unix: `:` splits them.
                ("C:\\x\\config", &["C", "\\x\\config"]),
                // No `~` expansion, as in kubectl.
                ("~/.kube/config:~/b", &["~/.kube/config", "~/b"]),
                // Relative entries, spaces and dots are kept verbatim.
                (
                    "rel/config: spaced/x :./y",
                    &["rel/config", " spaced/x ", "./y"],
                ),
            ],
        );
    }

    #[test]
    fn windows_table() {
        check(
            Platform::Windows,
            &[
                (
                    "C:\\Users\\me\\.kube\\config",
                    &["C:\\Users\\me\\.kube\\config"],
                ),
                ("C:/Users/me/.kube/config", &["C:/Users/me/.kube/config"]),
                (
                    "C:\\a\\config;D:\\b\\config",
                    &["C:\\a\\config", "D:\\b\\config"],
                ),
                // `:` between entries separates, drive colons do not.
                (
                    "C:\\a\\config:D:\\b\\config",
                    &["C:\\a\\config", "D:\\b\\config"],
                ),
                ("C:\\a;D:\\b:E:\\c", &["C:\\a", "D:\\b", "E:\\c"]),
                // Unix-style paths separated by `:` (WSL / Git Bash habit).
                ("/mnt/c/a:/mnt/d/b", &["/mnt/c/a", "/mnt/d/b"]),
                ("/c/a;/d/b", &["/c/a", "/d/b"]),
                // Empty entries and trailing/repeated separators.
                ("", &[]),
                (";", &[]),
                (";;C:\\a;;", &["C:\\a"]),
                ("C:\\a;", &["C:\\a"]),
                // UNC and relative paths.
                (
                    "\\\\srv\\share\\config;rel\\c",
                    &["\\\\srv\\share\\config", "rel\\c"],
                ),
                // Quotes group and are removed.
                ("\"C:\\a;b\\config\";D:\\c", &["C:\\a;b\\config", "D:\\c"]),
                // Multi-letter prefixes are not drives.
                ("ab:c", &["ab", "c"]),
                // No `~` expansion.
                ("~\\.kube\\config", &["~\\.kube\\config"]),
            ],
        );
    }

    #[test]
    fn single_path_is_the_same_on_both_platforms_when_unambiguous() {
        for platform in [Platform::Unix, Platform::Windows] {
            assert_eq!(
                split_kubeconfig("/etc/kube/config", platform),
                paths(&["/etc/kube/config"])
            );
        }
    }

    #[test]
    fn host_platform_matches_cfg() {
        assert_eq!(
            Platform::host(),
            if cfg!(windows) {
                Platform::Windows
            } else {
                Platform::Unix
            }
        );
        assert_eq!(Platform::default(), Platform::host());
    }

    #[test]
    fn os_variant_uses_the_given_platform_for_utf8() {
        assert_eq!(
            split_kubeconfig_os(OsStr::new("C:\\a;D:\\b"), Platform::Windows),
            paths(&["C:\\a", "D:\\b"])
        );
    }
}
