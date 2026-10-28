//! Where the CLIs are: the login shell's `PATH` ([`login_shell_path`]), merged with the app's
//! own and the usual install folders ([`SearchPath`]).
//!
//! An app started from Finder, the Dock or a desktop launcher gets a minimal `PATH` that
//! misses Homebrew, the Google Cloud SDK and the folders the user's shell profile adds, so
//! `aws` or `gcloud` would look missing. The runner asks the user's login shell once
//! (`$SHELL -lc 'printenv PATH'`, which works in sh, bash, zsh and fish) and caches the answer;
//! a shell that fails, prints nothing usable or hangs past its timeout is ignored.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::cli::Cli;

/// Folders searched after the login shell's and the app's `PATH`: where package managers put
/// the CLIs (the same list as the app's kubectl lookup).
pub const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/snap/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

/// Printed around `PATH` by the probe, so whatever a profile prints (a banner, a fortune) is
/// not mistaken for it.
const START: &str = "__OXIKUBE_PATH_START__";
const END: &str = "__OXIKUBE_PATH_END__";

/// Which shell the runner asks for the user's `PATH`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LoginShell {
    /// `$SHELL`, when set (the default).
    #[default]
    FromEnv,
    /// This shell.
    Program(PathBuf),
    /// Do not ask: use the app's own `PATH` and the usual folders.
    Disabled,
}

impl LoginShell {
    /// The shell to run, if any. Never one on Windows, which has no login shell `PATH`.
    pub(crate) fn program(&self) -> Option<PathBuf> {
        if cfg!(windows) {
            return None;
        }
        match self {
            Self::FromEnv => std::env::var_os("SHELL")
                .filter(|shell| !shell.is_empty())
                .map(PathBuf::from),
            Self::Program(path) => Some(path.clone()),
            Self::Disabled => None,
        }
    }
}

/// The `PATH` the user's login shell sets: `shell -lc` running `printenv PATH` between two
/// markers, with stdin closed, killed after `timeout`. `None` when the shell cannot be run,
/// fails, hangs or prints no absolute folder.
pub async fn login_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    let script = format!("echo {START}; printenv PATH; echo {END}");
    let mut command = Command::new(shell);
    command
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = command.spawn().ok()?;
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

/// The folders to look for a CLI in, in order, and the same list as a `PATH` value for the
/// child (so a CLI finds its own helpers: `gcloud` its Python, `aws` its SSO browser opener).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchPath {
    dirs: Vec<PathBuf>,
    joined: OsString,
}

impl SearchPath {
    /// The login shell's folders, then the app's, then [`EXTRA_DIRS`]; relative and repeated
    /// folders dropped.
    pub(crate) fn build(login: Option<&OsStr>, process: Option<&OsStr>) -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let listed = [login, process]
            .into_iter()
            .flatten()
            .flat_map(std::env::split_paths);
        let candidates = listed.chain(EXTRA_DIRS.iter().map(PathBuf::from));
        for dir in candidates {
            if dir.is_absolute() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        let joined = std::env::join_paths(&dirs).unwrap_or_default();
        Self { dirs, joined }
    }

    /// The `PATH` value for children.
    pub(crate) fn as_path_var(&self) -> &OsStr {
        &self.joined
    }

    /// The first executable for `cli` in the folders. A few `stat` calls; call it off the UI
    /// thread.
    pub(crate) fn find(&self, cli: Cli) -> Option<PathBuf> {
        let names = cli.file_names();
        self.dirs
            .iter()
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .find(|candidate| is_executable(candidate))
    }
}

/// `path` is a file this user may execute.
pub(crate) fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
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
        assert_eq!(parse_probe("/a/bin:/usr/bin"), None, "no markers");
        assert_eq!(parse_probe(&format!("{START}\n\n{END}")), None, "empty");
        assert_eq!(
            parse_probe(&format!("{START}\nrel/bin\n{END}")),
            None,
            "relative only"
        );
    }

    #[test]
    fn search_path_orders_login_then_process_then_extras_without_repeats() {
        let path = SearchPath::build(
            Some(OsStr::new("/login/bin:/usr/bin")),
            Some(OsStr::new("/usr/bin:relative:/proc/bin")),
        );
        let dirs: Vec<_> = path.dirs.iter().map(|d| d.display().to_string()).collect();
        assert_eq!(dirs[..3], ["/login/bin", "/usr/bin", "/proc/bin"]);
        assert!(dirs.iter().any(|d| d == "/opt/homebrew/bin"));
        assert_eq!(dirs.iter().filter(|d| *d == "/usr/bin").count(), 1);
        assert!(!dirs.iter().any(|d| d == "relative"));
        let fallback = SearchPath::build(None, None);
        assert_eq!(fallback.dirs.len(), EXTRA_DIRS.len());
    }

    #[test]
    fn disabled_and_explicit_shells() {
        assert_eq!(LoginShell::Disabled.program(), None);
        if !cfg!(windows) {
            let shell = LoginShell::Program("/bin/zsh".into()).program();
            assert_eq!(shell, Some(PathBuf::from("/bin/zsh")));
        }
    }
}
