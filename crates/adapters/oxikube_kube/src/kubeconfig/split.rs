//! Splitting the `KUBECONFIG` value into file paths.
//!
//! Pure: it takes the value as an argument and never reads the process environment, so tests
//! need no `set_var`. E03-S10 extends this with a `Platform` parameter for the `:` / `;` rules;
//! until then the host's own separator applies (`:` on Unix, `;` on Windows), through
//! [`std::env::split_paths`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Split a `KUBECONFIG` value into its paths, in order.
///
/// Empty entries are dropped (kubectl ignores empty file names). Duplicates are kept here; the
/// loader de-duplicates by canonical path.
pub fn split_kubeconfig_paths(value: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
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
}
