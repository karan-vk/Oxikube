//! Finding kubectl: [`KubectlLookup`] (where to look), [`PathLookup`] (the real one: `PATH` plus
//! the folders a GUI launch lacks) and [`Kubectl`], the answer cached for the UI to read.
//!
//! Looking costs a few `stat` calls, but the UI thread never makes them: the binary refreshes
//! the answer on a background task (at start-up, and again when a log view opens, so installing
//! kubectl while the app runs shows the action at the next view), and the toolbar reads the cached
//! answer.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

/// Where kubectl is, if anywhere. Blocking (it touches the file system): call it off the UI
/// thread. A closure `Fn() -> Option<PathBuf>` is a lookup too, which is what the tests use.
pub trait KubectlLookup: Send + Sync + 'static {
    /// The absolute path of the kubectl executable, `None` when there is none.
    fn find(&self) -> Option<PathBuf>;
}

impl<F> KubectlLookup for F
where
    F: Fn() -> Option<PathBuf> + Send + Sync + 'static,
{
    fn find(&self) -> Option<PathBuf> {
        self()
    }
}

/// Looks for an executable `kubectl` in a list of folders.
#[derive(Debug, Clone, Default)]
pub struct PathLookup {
    dirs: Vec<PathBuf>,
}

/// Folders a kubectl installed by a package manager lives in that a GUI launch (Finder, a
/// desktop entry) often leaves out of `PATH`, which only a login shell sets.
const EXTRA_DIRS: [&str; 5] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/snap/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

impl PathLookup {
    /// Looks in these folders, in order.
    pub fn in_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            dirs: dirs.into_iter().collect(),
        }
    }

    /// Looks in `PATH`, then the usual install folders.
    pub fn system() -> Self {
        Self::from_path_var(std::env::var_os("PATH"))
    }

    /// [`system`](Self::system) over a given `PATH` value.
    pub fn from_path_var(path: Option<OsString>) -> Self {
        let mut dirs: Vec<PathBuf> = path
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
        Self { dirs }
    }
}

impl KubectlLookup for PathLookup {
    fn find(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) {
            "kubectl.exe"
        } else {
            "kubectl"
        };
        self.dirs
            .iter()
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|candidate| is_executable(candidate))
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Whether kubectl is installed, and where: a cheap, shared handle on the last lookup.
///
/// Nothing is known until the first [`refresh`](Self::refresh), and "not known" reads as "not
/// installed": the action stays hidden rather than flashing in and out.
#[derive(Clone)]
pub struct Kubectl {
    lookup: Arc<dyn KubectlLookup>,
    found: Arc<RwLock<Option<PathBuf>>>,
}

impl std::fmt::Debug for Kubectl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kubectl")
            .field("found", &*self.found.read())
            .finish_non_exhaustive()
    }
}

impl Kubectl {
    /// A handle that looks with `lookup`.
    pub fn new(lookup: impl KubectlLookup) -> Self {
        Self {
            lookup: Arc::new(lookup),
            found: Arc::default(),
        }
    }

    /// A handle that looks on this machine ([`PathLookup::system`]).
    pub fn system() -> Self {
        Self::new(PathLookup::system())
    }

    /// Looks again and keeps the answer. Blocking: run it on a background task. Returns whether
    /// kubectl was found.
    pub fn refresh(&self) -> bool {
        let found = self.lookup.find();
        let present = found.is_some();
        *self.found.write() = found;
        present
    }

    /// The kubectl found by the last [`refresh`](Self::refresh).
    pub fn path(&self) -> Option<PathBuf> {
        self.found.read().clone()
    }

    /// Whether the last [`refresh`](Self::refresh) found kubectl.
    pub fn is_available(&self) -> bool {
        self.found.read().is_some()
    }
}
