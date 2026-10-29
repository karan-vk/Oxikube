//! Where the binaries are: the login shell's `PATH`, merged with the app's own and the usual
//! install folders.
//!
//! An app started from Finder, the Dock or a launcher gets a minimal `PATH` that misses
//! Homebrew and the folders a shell profile adds, so `kustomize` would look missing. The
//! adapter asks the user's login shell once (`$SHELL -l -c 'printenv PATH'`, which works in
//! sh, bash, zsh and fish) and keeps the answer. Copied from `oxikube_cloud::process::path`
//! (adapters never depend on each other).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

/// Folders searched after the login shell's and the app's `PATH`.
pub const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/snap/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

/// Printed around `PATH` by the probe, so a profile's banner is not mistaken for it.
const START: &str = "__OXIKUBE_PATH_START__";
const END: &str = "__OXIKUBE_PATH_END__";

/// The `PATH` the user's login shell sets, `None` when the shell cannot be run, fails, hangs
/// past `timeout` or prints no absolute folder. Never on Windows.
pub async fn login_shell_path(timeout: Duration) -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var_os("SHELL").filter(|shell| !shell.is_empty())?;
    let script = format!("echo {START}; printenv PATH; echo {END}");
    let child = Command::new(shell)
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    // On timeout the future, and with it the child, is dropped: `kill_on_drop` kills it.
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_probe(&String::from_utf8_lossy(&output.stdout))
}

/// The `PATH` between the markers, when it names at least one absolute folder.
fn parse_probe(stdout: &str) -> Option<OsString> {
    let after = stdout.split_once(START)?.1;
    let path = after.split_once(END)?.0.trim();
    let has_dir = std::env::split_paths(path).any(|dir| dir.is_absolute());
    has_dir.then(|| OsString::from(path))
}

/// The ordered, de-duplicated folders to look in, and the same list as a `PATH` value for the
/// child (so `kubectl` finds its own helpers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    dirs: Vec<PathBuf>,
    joined: OsString,
}

impl Folders {
    /// The login shell's folders, then the app's, then [`EXTRA_DIRS`]; relative and repeated
    /// folders dropped.
    pub fn build(login: Option<&OsStr>, process: Option<&OsStr>) -> Self {
        let listed = [login, process]
            .into_iter()
            .flatten()
            .flat_map(std::env::split_paths);
        Self::from_dirs(listed.chain(EXTRA_DIRS.iter().map(PathBuf::from)))
    }

    /// Exactly the folders of the `PATH`-style list `list`.
    pub fn fixed(list: &OsStr) -> Self {
        Self::from_dirs(std::env::split_paths(list))
    }

    fn from_dirs(candidates: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for dir in candidates {
            if dir.is_absolute() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        let joined = std::env::join_paths(&dirs).unwrap_or_default();
        Self { dirs, joined }
    }

    /// The `PATH` value for children.
    pub fn as_path_var(&self) -> &OsStr {
        &self.joined
    }

    /// The first executable called `name` (with `.exe` on Windows) in the folders. A few
    /// `stat` calls; call it off the UI thread.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        let file = if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        };
        self.dirs
            .iter()
            .map(|dir| dir.join(&file))
            .find(|candidate| is_executable(candidate))
    }
}

/// `path` is a file this user may execute.
pub fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_output_is_read_between_the_markers() {
        let out = format!("Welcome!\n{START}\n/a/bin:/usr/bin\n{END}\n");
        assert_eq!(parse_probe(&out), Some(OsString::from("/a/bin:/usr/bin")));
        assert_eq!(parse_probe("/a/bin"), None, "no markers");
        assert_eq!(parse_probe(&format!("{START}\nrel/bin\n{END}")), None);
    }

    #[test]
    fn folders_order_login_then_process_then_extras_without_repeats() {
        let folders = Folders::build(
            Some(OsStr::new("/login/bin:/usr/bin")),
            Some(OsStr::new("/usr/bin:relative:/proc/bin")),
        );
        let dirs: Vec<_> = folders
            .dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        assert_eq!(dirs[..3], ["/login/bin", "/usr/bin", "/proc/bin"]);
        assert_eq!(dirs.iter().filter(|d| *d == "/usr/bin").count(), 1);
        assert!(!dirs.iter().any(|d| d == "relative"));
    }

    #[test]
    fn fixed_folders_are_exactly_the_list() {
        let folders = Folders::fixed(OsStr::new("/one:/two"));
        assert_eq!(folders.dirs, [PathBuf::from("/one"), PathBuf::from("/two")]);
    }
}
