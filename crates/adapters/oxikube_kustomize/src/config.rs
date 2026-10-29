//! [`KustomizeConfig`]: what the user configures for the kustomize adapter.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// How long a build may run before the process is killed.
pub const DEFAULT_BUILD_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the version probes and the login-shell probe may run.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the binaries are looked up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SearchPath {
    /// The user's login shell `PATH`, then the app's own, then the usual install folders.
    #[default]
    Auto,
    /// Exactly these folders (a `PATH`-style list); tests use it to control detection.
    Fixed(OsString),
}

/// The adapter's configuration (the `kustomize` settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KustomizeConfig {
    /// The binary to use (`kustomize.path`): a `kubectl` file name selects `kubectl kustomize`,
    /// anything else is taken as a standalone kustomize. `None` autodetects. A path that is
    /// not an executable file means "not installed"; it is never searched for elsewhere.
    pub path: Option<PathBuf>,
    /// Deadline of one build.
    pub build_timeout: Duration,
    /// Deadline of each version probe.
    pub probe_timeout: Duration,
    /// Where binaries are looked up.
    pub search_path: SearchPath,
}

impl Default for KustomizeConfig {
    /// Autodetect, a 60 s build deadline and a 10 s probe deadline.
    fn default() -> Self {
        Self {
            path: None,
            build_timeout: DEFAULT_BUILD_TIMEOUT,
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
            search_path: SearchPath::Auto,
        }
    }
}
