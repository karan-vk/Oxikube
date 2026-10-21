//! [`HelmCliConfig`] and [`HelmTimeouts`]: what the user configures for the `helm` CLI adapter.

use std::path::PathBuf;
use std::time::Duration;

/// How long each kind of `helm` command may run before the process is killed and the call
/// fails with `Timeout`.
///
/// Mutations (install, upgrade, rollback, uninstall) run for the request's own `timeout`
/// (Helm's `--timeout`) plus [`mutation_grace`](Self::mutation_grace), so Helm reports its own
/// timeout first and the adapter only kills a process that hangs past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelmTimeouts {
    /// `helm version` while detecting the binary.
    pub probe: Duration,
    /// Commands that read local state or the cluster: `list`, `history`, `get`, `repo list`,
    /// `search repo`.
    pub read: Duration,
    /// Commands that may download a chart or an index: `show`, `template`, `repo add`,
    /// `repo update`.
    pub fetch: Duration,
    /// Added to a mutation's own timeout.
    pub mutation_grace: Duration,
}

impl Default for HelmTimeouts {
    /// 10 s probe, 30 s reads, 120 s fetches, 60 s grace on mutations.
    fn default() -> Self {
        Self {
            probe: Duration::from_secs(10),
            read: Duration::from_secs(30),
            fetch: Duration::from_secs(120),
            mutation_grace: Duration::from_secs(60),
        }
    }
}

/// The adapter's configuration (the `helm` settings, E16-S11).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelmCliConfig {
    /// The `helm` binary (setting `helm.path`); `None` looks `helm` up on `PATH` and in the
    /// usual install directories.
    pub path: Option<PathBuf>,
    /// Per-command deadlines.
    pub timeouts: HelmTimeouts,
}
