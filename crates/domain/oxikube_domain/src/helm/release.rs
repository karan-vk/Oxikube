//! Releases, revisions and chart references.

use std::fmt;
use std::sync::Arc;

use jiff::Timestamp;

use super::values::Values;
use crate::ids::ClusterId;

/// Identifies a release: Helm release names are unique per namespace.
///
/// Displays as `namespace/name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReleaseRef {
    /// The cluster holding the release.
    pub cluster: ClusterId,
    /// The release's namespace.
    pub namespace: String,
    /// The release name.
    pub name: String,
}

impl ReleaseRef {
    /// A reference to `name` in `namespace` of `cluster`.
    pub fn new(cluster: ClusterId, namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            cluster,
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

impl fmt::Display for ReleaseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// A release's lifecycle state, as Helm reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseStatus {
    /// Helm does not know the state.
    Unknown,
    /// Installed and live.
    Deployed,
    /// Removed but history kept.
    Uninstalled,
    /// Replaced by a newer revision.
    Superseded,
    /// The last operation failed.
    Failed,
    /// An uninstall is in progress.
    Uninstalling,
    /// An install is in progress.
    PendingInstall,
    /// An upgrade is in progress.
    PendingUpgrade,
    /// A rollback is in progress.
    PendingRollback,
}

impl ReleaseStatus {
    /// Every status, in declaration order.
    pub const ALL: [ReleaseStatus; 9] = [
        Self::Unknown,
        Self::Deployed,
        Self::Uninstalled,
        Self::Superseded,
        Self::Failed,
        Self::Uninstalling,
        Self::PendingInstall,
        Self::PendingUpgrade,
        Self::PendingRollback,
    ];

    /// Helm's spelling (`pending-install`), as in the release `status` label and
    /// `helm list -o json`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Deployed => "deployed",
            Self::Uninstalled => "uninstalled",
            Self::Superseded => "superseded",
            Self::Failed => "failed",
            Self::Uninstalling => "uninstalling",
            Self::PendingInstall => "pending-install",
            Self::PendingUpgrade => "pending-upgrade",
            Self::PendingRollback => "pending-rollback",
        }
    }

    /// Parses Helm's spelling, case-insensitively. Anything unrecognised is
    /// [`Unknown`](Self::Unknown): a newer Helm must not break the list.
    pub fn parse(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str().eq_ignore_ascii_case(s.trim()))
            .unwrap_or(Self::Unknown)
    }

    /// `true` while an operation is in flight (`pending-*`, `uninstalling`): the revision
    /// is not final and mutating it again would race.
    pub const fn is_pending(self) -> bool {
        matches!(
            self,
            Self::Uninstalling
                | Self::PendingInstall
                | Self::PendingUpgrade
                | Self::PendingRollback
        )
    }
}

impl fmt::Display for ReleaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A chart, optionally with the repository it comes from.
///
/// For an installed release `repo` is `None` (the release record does not keep it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChartRef {
    /// The repository alias (`bitnami`), when known.
    pub repo: Option<String>,
    /// Chart name (`nginx`).
    pub name: String,
    /// Chart version (`15.4.0`). Empty means "latest" in a request.
    pub version: String,
}

impl ChartRef {
    /// A chart without a repository.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            repo: None,
            name: name.into(),
            version: version.into(),
        }
    }

    /// The same chart in repository `repo`.
    #[must_use]
    pub fn in_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    /// The reference the `helm` CLI takes: `repo/name`, or `name` without a repository.
    pub fn reference(&self) -> String {
        match &self.repo {
            Some(repo) => format!("{repo}/{}", self.name),
            None => self.name.clone(),
        }
    }
}

impl fmt::Display for ChartRef {
    /// `name-version`, the way `helm list` prints the chart column.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.version.is_empty() {
            f.write_str(&self.name)
        } else {
            write!(f, "{}-{}", self.name, self.version)
        }
    }
}

/// One revision of a release, with the data a list needs.
///
/// `list_releases` fills the cheap fields only: [`notes`](Self::notes) is `None` and both
/// value trees are empty. `HelmPort::notes` / `values` / `manifest` fetch those on demand,
/// so a 500-release list never decodes (or clones) the big payloads. Large strings are
/// shared (`Arc`), so cloning a `Release` is cheap.
#[derive(Clone, PartialEq)]
pub struct Release {
    /// Which release this revision belongs to.
    pub release: ReleaseRef,
    /// The chart it was installed from.
    pub chart: ChartRef,
    /// Revision number, starting at 1.
    pub revision: u32,
    /// State of this revision.
    pub status: ReleaseStatus,
    /// Version of the packaged application, if the chart declares one.
    pub app_version: Option<String>,
    /// When this revision was last updated.
    pub updated: Timestamp,
    /// `NOTES.txt` as rendered, when loaded.
    pub notes: Option<Arc<str>>,
    /// The values the user supplied, when loaded.
    pub values_user: Values,
    /// The chart defaults merged with the user's values, when loaded.
    pub values_computed: Values,
}

impl fmt::Debug for Release {
    /// Redacts the rendered notes (charts often echo credentials there); values redact themselves.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Release")
            .field("release", &self.release)
            .field("chart", &self.chart)
            .field("revision", &self.revision)
            .field("status", &self.status)
            .field("app_version", &self.app_version)
            .field("updated", &self.updated)
            .field(
                "notes",
                &self
                    .notes
                    .as_ref()
                    .map(|n| format!("<{} bytes, redacted>", n.len())),
            )
            .field("values_user", &self.values_user)
            .field("values_computed", &self.values_computed)
            .finish()
    }
}

impl Release {
    /// A list-shaped release: no notes, empty values.
    pub fn summary(
        release: ReleaseRef,
        chart: ChartRef,
        revision: u32,
        status: ReleaseStatus,
        updated: Timestamp,
    ) -> Self {
        Self {
            release,
            chart,
            revision,
            status,
            app_version: None,
            updated,
            notes: None,
            values_user: Values::default(),
            values_computed: Values::default(),
        }
    }
}

/// One entry of a release's history (`helm history`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Revision number, starting at 1.
    pub revision: u32,
    /// State of this revision.
    pub status: ReleaseStatus,
    /// The chart of this revision.
    pub chart: ChartRef,
    /// Version of the packaged application, if the chart declares one.
    pub app_version: Option<String>,
    /// When this revision was last updated.
    pub updated: Timestamp,
    /// Helm's description (`Upgrade complete`, `Rollback to 2`).
    pub description: String,
}
