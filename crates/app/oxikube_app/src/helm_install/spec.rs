//! [`InstallSpec`], [`InstallPreview`], [`InstallError`] and what the guard says about a
//! `helm::Install` (its confirmation text, audit target and audit detail).

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::command::Command;
use oxikube_domain::helm::{
    ChartRef, HelmError, InstallRequest, ManifestSummary, ReleaseRef, ValuesYaml,
    request::DEFAULT_TIMEOUT,
};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};

/// The kind an install is audited against: a release is no Kubernetes object, so the record names
/// the synthetic `helm.sh/v3 Release` in the release's namespace.
pub fn release_gvk() -> Gvk {
    Gvk::new("helm.sh", "v3", "Release")
}

/// One install as the tab describes it: the form's fields and the values text. The same spec makes
/// the preview's dry run and the install, so what was previewed is what is confirmed.
///
/// `Debug` never shows the values ([`ValuesYaml`] redacts them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSpec {
    /// The cluster to install into.
    pub cluster: ClusterId,
    /// The release name.
    pub release: String,
    /// The namespace.
    pub namespace: String,
    /// Create the namespace when it is missing.
    pub create_namespace: bool,
    /// The chart; an empty `version` is the newest.
    pub chart: ChartRef,
    /// The values YAML (the editor's text).
    pub values: ValuesYaml,
    /// Wait for the resources to become ready.
    pub wait: bool,
    /// Helm's `--timeout`; `None` is Helm's default.
    pub timeout: Option<Duration>,
}

impl InstallSpec {
    /// A spec installing `chart` as `release` in `namespace` with the chart's defaults.
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
            values: ValuesYaml::default(),
            wait: false,
            timeout: None,
        }
    }

    /// The `helm::Install` command for this spec.
    pub fn command(&self) -> Command {
        Command::HelmInstall {
            cluster: self.cluster.clone(),
            release: self.release.clone(),
            namespace: self.namespace.clone(),
            create_namespace: self.create_namespace,
            repo: self.chart.repo.clone(),
            chart: self.chart.name.clone(),
            version: (!self.chart.version.is_empty()).then(|| self.chart.version.clone()),
            values: self.values.clone(),
            wait: self.wait,
            timeout_secs: self.timeout.map(|t| t.as_secs()),
        }
    }

    /// The spec a `helm::Install` command describes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        let Command::HelmInstall {
            cluster,
            release,
            namespace,
            create_namespace,
            repo,
            chart,
            version,
            values,
            wait,
            timeout_secs,
        } = command
        else {
            return None;
        };
        let chart = ChartRef::new(chart, version.clone().unwrap_or_default());
        Some(Self {
            cluster: cluster.clone(),
            release: release.clone(),
            namespace: namespace.clone(),
            create_namespace: *create_namespace,
            chart: match repo {
                Some(repo) => chart.in_repo(repo),
                None => chart,
            },
            values: values.clone(),
            wait: *wait,
            timeout: timeout_secs.map(Duration::from_secs),
        })
    }

    /// The release this spec makes.
    pub fn release_ref(&self) -> ReleaseRef {
        ReleaseRef::new(self.cluster.clone(), &self.namespace, &self.release)
    }

    /// The port's request: the values parsed, `dry_run` as asked.
    ///
    /// # Errors
    ///
    /// [`HelmError::Parse`] when the values are not a YAML map.
    pub fn request(&self, dry_run: bool) -> Result<InstallRequest, HelmError> {
        let mut request = InstallRequest::new(
            self.cluster.clone(),
            &self.release,
            &self.namespace,
            self.chart.clone(),
        );
        request.create_namespace = self.create_namespace;
        request.values = self.values.parse()?;
        request.dry_run = dry_run;
        request.wait = self.wait;
        request.timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT);
        Ok(request)
    }
}

/// What the preview showed: the rendered manifest and the server's verdict, for the spec it was
/// made from. [`InstallFlow::ask`](super::InstallFlow::ask) takes it, so only a previewed spec can
/// be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPreview {
    /// The spec the preview was made from.
    pub spec: InstallSpec,
    /// The manifest `helm template` rendered, with Secret data and secret-looking fields masked.
    pub manifest: Arc<str>,
    /// The manifest's objects per kind.
    pub summary: ManifestSummary,
    /// Why the server's dry run refused the install (a name in use, a namespace that does not
    /// exist, a rejected object); `None` when it passed.
    pub server_error: Option<String>,
}

impl InstallPreview {
    /// Whether the install may be asked for: the server's dry run passed.
    pub fn can_install(&self) -> bool {
        self.server_error.is_none()
    }
}

/// Why a preview or an install did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// The cluster is in read-only mode; nothing was sent.
    #[error("{context} is read-only: installing is turned off")]
    ReadOnly {
        /// The cluster's context name.
        context: String,
    },
    /// The form or the values are not valid; nothing was sent.
    #[error("{0}")]
    Invalid(String),
    /// The confirmation was declined, expired or asked for more than one click.
    #[error("the install was not confirmed")]
    Unconfirmed,
    /// `helm` (or the cluster) refused or failed; the text is redacted.
    #[error("{0}")]
    Failed(String),
}

fn chart_label(command: &Command) -> Option<(String, String)> {
    let Command::HelmInstall {
        repo,
        chart,
        version,
        ..
    } = command
    else {
        return None;
    };
    let reference = match repo {
        Some(repo) => format!("{repo}/{chart}"),
        None => chart.clone(),
    };
    let version = version.clone().unwrap_or_else(|| "latest".to_owned());
    Some((reference, version))
}

/// The confirmation text of `helm::Install`: chart, version, release, namespace and cluster.
/// Never the values.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    let (chart, version) = chart_label(command)?;
    let Command::HelmInstall {
        release,
        namespace,
        create_namespace,
        ..
    } = command
    else {
        return None;
    };
    let created = if *create_namespace {
        " (created if missing)"
    } else {
        ""
    };
    Some(format!(
        "Install chart {chart} {version} as release {release} in namespace {namespace}{created} on \
         {context}"
    ))
}

/// The audit target of `helm::Install`: the release, as `helm.sh/v3 Release <namespace>/<name>`.
pub(crate) fn audit_target(command: &Command, cluster: &ClusterId) -> Option<ResourceRef> {
    let Command::HelmInstall {
        release, namespace, ..
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

/// The audit detail of `helm::Install`: the chart, its version and the flags; whether values were
/// given, never what they are.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    let (chart, version) = chart_label(command)?;
    let Command::HelmInstall {
        create_namespace,
        values,
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
        "chart={chart} version={version} create_namespace={create_namespace} wait={wait} \
         values={values}"
    ))
}
