//! [`UpgradeSpec`], [`UpgradePreview`], [`UpgradeError`] and what the guard says about a
//! `helm::Upgrade` (its confirmation text, audit target and audit detail).

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::command::Command;
use oxikube_domain::helm::{
    ChartRef, HelmError, ManifestSummary, ReleaseRef, ResourceChanges, UpgradeRequest, ValuesYaml,
    request::DEFAULT_TIMEOUT,
};
use oxikube_domain::ids::{ClusterId, ResourceRef};

use crate::helm_install::{InstallError, release_gvk};

/// One upgrade as the tab describes it: the release, the chart to move to, the values text and the
/// flags. The same spec makes the preview's dry run and the upgrade, so what was previewed is what
/// is confirmed. `Debug` never shows the values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeSpec {
    /// The release to upgrade.
    pub release: ReleaseRef,
    /// The chart to upgrade to; an empty `version` is the newest.
    pub chart: ChartRef,
    /// The values YAML (the editor's text).
    pub values: ValuesYaml,
    /// Keep the previous revision's values under these (`--reuse-values`).
    pub reuse_values: bool,
    /// Wait for the resources to become ready.
    pub wait: bool,
    /// Helm's `--timeout`; `None` is Helm's default.
    pub timeout: Option<Duration>,
}

impl UpgradeSpec {
    /// A spec upgrading `release` to `chart` with the chart's defaults.
    pub fn new(release: ReleaseRef, chart: ChartRef) -> Self {
        Self {
            release,
            chart,
            values: ValuesYaml::default(),
            reuse_values: false,
            wait: false,
            timeout: None,
        }
    }

    /// The `helm::Upgrade` command for this spec.
    pub fn command(&self) -> Command {
        Command::HelmUpgrade {
            cluster: self.release.cluster.clone(),
            namespace: self.release.namespace.clone(),
            release: self.release.name.clone(),
            repo: self.chart.repo.clone(),
            chart: self.chart.name.clone(),
            version: (!self.chart.version.is_empty()).then(|| self.chart.version.clone()),
            values: self.values.clone(),
            reuse_values: self.reuse_values,
            wait: self.wait,
            timeout_secs: self.timeout.map(|t| t.as_secs()),
        }
    }

    /// The spec a `helm::Upgrade` command describes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        let Command::HelmUpgrade {
            cluster,
            namespace,
            release,
            repo,
            chart,
            version,
            values,
            reuse_values,
            wait,
            timeout_secs,
        } = command
        else {
            return None;
        };
        let chart = ChartRef::new(chart, version.clone().unwrap_or_default());
        Some(Self {
            release: ReleaseRef::new(cluster.clone(), namespace, release),
            chart: match repo {
                Some(repo) => chart.in_repo(repo),
                None => chart,
            },
            values: values.clone(),
            reuse_values: *reuse_values,
            wait: *wait,
            timeout: timeout_secs.map(Duration::from_secs),
        })
    }

    /// The cluster.
    pub fn cluster(&self) -> &ClusterId {
        &self.release.cluster
    }

    /// The port's request: the values parsed, `dry_run` as asked.
    ///
    /// # Errors
    ///
    /// [`HelmError::Parse`] when the values are not a YAML map.
    pub fn request(&self, dry_run: bool) -> Result<UpgradeRequest, HelmError> {
        let mut request = UpgradeRequest::new(self.release.clone(), self.chart.clone());
        request.values = self.values.parse()?;
        request.reuse_values = self.reuse_values;
        request.dry_run = dry_run;
        request.wait = self.wait;
        request.timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT);
        Ok(request)
    }
}

/// What the preview showed: the rendered manifest, the objects it changes, the server's verdict and
/// the chart the release is on now, for the spec it was made from.
/// [`UpgradeFlow::ask`](super::UpgradeFlow::ask) takes it, so only a previewed spec is upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePreview {
    /// The spec the preview was made from.
    pub spec: UpgradeSpec,
    /// The manifest `helm template` rendered, Secret data masked.
    pub manifest: Arc<str>,
    /// The manifest's objects per kind.
    pub summary: ManifestSummary,
    /// Objects added, removed and changed against the release's current manifest.
    pub changes: ResourceChanges,
    /// The chart the release runs now (`nginx-15.0.0`), for the confirmation and the audit.
    pub from_chart: String,
    /// Why the server's dry run refused the upgrade; `None` when it passed.
    pub server_error: Option<String>,
}

impl UpgradePreview {
    /// Whether the upgrade may be asked for: the server's dry run passed.
    pub fn can_upgrade(&self) -> bool {
        self.server_error.is_none()
    }
}

/// Why a preview or an upgrade did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpgradeError {
    /// The cluster is in read-only mode; nothing was sent.
    #[error("{context} is read-only: upgrading is turned off")]
    ReadOnly {
        /// The cluster's context name.
        context: String,
    },
    /// The form or the values are not valid; nothing was sent.
    #[error("{0}")]
    Invalid(String),
    /// The confirmation was declined, expired or asked for more than one click.
    #[error("the upgrade was not confirmed")]
    Unconfirmed,
    /// `helm` (or the cluster) refused or failed; the text is redacted. The release may hold a
    /// `failed` revision: History shows it and Rollback restores an earlier one.
    #[error("{0}")]
    Failed(String),
}

impl From<InstallError> for UpgradeError {
    fn from(error: InstallError) -> Self {
        match error {
            InstallError::ReadOnly { context } => Self::ReadOnly { context },
            InstallError::Invalid(why) => Self::Invalid(why),
            InstallError::Unconfirmed => Self::Unconfirmed,
            InstallError::Failed(why) => Self::Failed(why),
        }
    }
}

fn chart_label(chart: &str, repo: &Option<String>) -> String {
    match repo {
        Some(repo) => format!("{repo}/{chart}"),
        None => chart.to_owned(),
    }
}

/// The confirmation text of `helm::Upgrade`: release, namespace, target chart and version, the
/// `--reuse-values` flag and the cluster. Never the values.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    let Command::HelmUpgrade {
        namespace,
        release,
        repo,
        chart,
        version,
        reuse_values,
        ..
    } = command
    else {
        return None;
    };
    let version = version.as_deref().unwrap_or("latest");
    let reuse = if *reuse_values {
        ", reusing the previous values"
    } else {
        ""
    };
    Some(format!(
        "Upgrade release {release} in namespace {namespace} to chart {} {version}{reuse} on \
         {context}",
        chart_label(chart, repo)
    ))
}

/// The audit target of `helm::Upgrade`: `helm.sh/v3 Release <namespace>/<name>`.
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    let Command::HelmUpgrade {
        namespace, release, ..
    } = command
    else {
        return None;
    };
    Some(ResourceRef::namespaced(
        cluster.clone(),
        release_gvk(),
        namespace.as_str(),
        release.as_str(),
    ))
}

/// The audit detail of `helm::Upgrade`: the target chart and version, `reuse_values` and the other
/// flags; whether values were given, never what they are. The handler adds `from=` (the chart the
/// release ran).
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    let Command::HelmUpgrade {
        repo,
        chart,
        version,
        values,
        reuse_values,
        wait,
        ..
    } = command
    else {
        return None;
    };
    let values = if values.is_blank() {
        "defaults"
    } else {
        "given"
    };
    Some(format!(
        "chart={} version={} reuse_values={reuse_values} wait={wait} values={values}",
        chart_label(chart, repo),
        version.as_deref().unwrap_or("latest"),
    ))
}
