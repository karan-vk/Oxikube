//! Requests for the mutating Helm operations, and the streamed output of install/upgrade.

use std::fmt;
use std::time::Duration;

use super::release::{ChartRef, ReleaseRef};
use super::values::Values;
use crate::ids::ClusterId;

/// The default `--timeout` of install, upgrade and rollback.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Install a chart as a new release; also the input of `template` (rendering only).
#[derive(Clone, PartialEq)]
pub struct InstallRequest {
    /// The cluster to install into.
    pub cluster: ClusterId,
    /// Release name.
    pub release: String,
    /// Target namespace.
    pub namespace: String,
    /// Create the namespace when missing (`--create-namespace`).
    pub create_namespace: bool,
    /// The chart; an empty `version` means the latest.
    pub chart: ChartRef,
    /// Values overriding the chart defaults.
    pub values: Values,
    /// Render and validate against the server without installing (`--dry-run`).
    pub dry_run: bool,
    /// Wait for resources to become ready (`--wait`).
    pub wait: bool,
    /// How long to wait for the operation.
    pub timeout: Duration,
}

impl InstallRequest {
    /// A request installing `chart` as `release` in `namespace`, with defaults: no
    /// namespace creation, no dry run, no wait, [`DEFAULT_TIMEOUT`], empty values.
    pub fn new(
        cluster: ClusterId,
        release: impl Into<String>,
        namespace: impl Into<String>,
        chart: ChartRef,
    ) -> Self {
        Self {
            cluster,
            release: release.into(),
            namespace: namespace.into(),
            create_namespace: false,
            chart,
            values: Values::default(),
            dry_run: false,
            wait: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallRequest")
            .field("release", &self.release)
            .field("namespace", &self.namespace)
            .field("chart", &self.chart)
            .field("dry_run", &self.dry_run)
            .field("values", &self.values)
            .finish_non_exhaustive()
    }
}

/// Upgrade an existing release.
#[derive(Clone, PartialEq)]
pub struct UpgradeRequest {
    /// The release to upgrade.
    pub release: ReleaseRef,
    /// The chart to upgrade to; an empty `version` means the latest.
    pub chart: ChartRef,
    /// The values to apply.
    pub values: Values,
    /// Start from the previous revision's values (`--reuse-values`) instead of the new
    /// chart's defaults.
    pub reuse_values: bool,
    /// Render and validate without upgrading (`--dry-run`).
    pub dry_run: bool,
    /// Wait for resources to become ready (`--wait`).
    pub wait: bool,
    /// How long to wait for the operation.
    pub timeout: Duration,
}

impl UpgradeRequest {
    /// A request upgrading `release` to `chart`, with defaults: values replaced, no dry
    /// run, no wait, [`DEFAULT_TIMEOUT`].
    pub fn new(release: ReleaseRef, chart: ChartRef) -> Self {
        Self {
            release,
            chart,
            values: Values::default(),
            reuse_values: false,
            dry_run: false,
            wait: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl fmt::Debug for UpgradeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpgradeRequest")
            .field("release", &self.release)
            .field("chart", &self.chart)
            .field("reuse_values", &self.reuse_values)
            .field("dry_run", &self.dry_run)
            .field("values", &self.values)
            .finish_non_exhaustive()
    }
}

/// Roll a release back to an earlier revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackRequest {
    /// The release.
    pub release: ReleaseRef,
    /// The revision to restore.
    pub revision: u32,
    /// Simulate the rollback (`--dry-run`).
    pub dry_run: bool,
    /// Wait for resources to become ready (`--wait`).
    pub wait: bool,
    /// How long to wait for the operation.
    pub timeout: Duration,
}

impl RollbackRequest {
    /// A request restoring `revision` of `release`.
    pub fn new(release: ReleaseRef, revision: u32) -> Self {
        Self {
            release,
            revision,
            dry_run: false,
            wait: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Uninstall a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallRequest {
    /// The release.
    pub release: ReleaseRef,
    /// Keep the release history (`--keep-history`), so it can be rolled back.
    pub keep_history: bool,
    /// Simulate the uninstall (`--dry-run`).
    pub dry_run: bool,
    /// How long to wait for the operation.
    pub timeout: Duration,
}

impl UninstallRequest {
    /// A request removing `release` and its history.
    pub fn new(release: ReleaseRef) -> Self {
        Self {
            release,
            keep_history: false,
            dry_run: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Which stream of the `helm` process a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// Standard output.
    Stdout,
    /// Standard error (progress and warnings; already redacted by the adapter).
    Stderr,
}

/// One line of install/upgrade progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    /// Where the line came from.
    pub kind: OutputKind,
    /// The line, without its newline, redacted.
    pub text: String,
}

impl OutputLine {
    /// A standard-output line.
    pub fn stdout(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stdout,
            text: text.into(),
        }
    }

    /// A standard-error line.
    pub fn stderr(text: impl Into<String>) -> Self {
        Self {
            kind: OutputKind::Stderr,
            text: text.into(),
        }
    }
}
