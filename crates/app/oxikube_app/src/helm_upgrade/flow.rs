//! [`UpgradeFlow`]: the upgrade tab's side of `helm::Upgrade`: what the release is now, preview,
//! ask, upgrade, follow.

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::audit::Initiator;
use oxikube_domain::helm::{
    ChartRef, ManifestSummary, Release, ReleaseRef, ReleaseStatus, ResourceChanges, ValuesYaml,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use serde_json::Value;

use super::spec::{UpgradeError, UpgradePreview, UpgradeSpec};
use crate::CommandBus;
use crate::command_bus::{DispatchContext, Outcome};
use crate::guard::{Confirmation, ConfirmationRequest};
use crate::helm_install::{ReleaseProgress, classify, rehearse};
use crate::helm_releases::ReleaseSource;

/// A release as the upgrade tab starts from it: what it runs now and the values it was given.
///
/// `Debug` never shows the values.
#[derive(Clone, PartialEq, Eq)]
pub struct CurrentRelease {
    /// The release.
    pub release: ReleaseRef,
    /// The chart and version it runs (the repository is not recorded in a release).
    pub chart: ChartRef,
    /// The chart's application version.
    pub app_version: Option<String>,
    /// The latest revision.
    pub revision: u32,
    /// Its status (`failed` after a failed upgrade).
    pub status: ReleaseStatus,
    /// The user-supplied values of the latest revision, as YAML. May hold secrets.
    pub values: ValuesYaml,
}

impl std::fmt::Debug for CurrentRelease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentRelease")
            .field("release", &self.release)
            .field("chart", &self.chart)
            .field("revision", &self.revision)
            .field("status", &self.status)
            .field("values", &self.values)
            .finish_non_exhaustive()
    }
}

/// Reads releases and previews, confirms and runs upgrades for the upgrade tab, every step of the
/// mutation a `helm::Upgrade` dispatch through the guard. Cheap to clone. Plain async: the tab runs
/// the futures on the Tokio bridge (`spawn_kube`).
#[derive(Clone)]
pub struct UpgradeFlow {
    bus: CommandBus,
    helm: Arc<dyn HelmPort>,
    /// The sessions' native readers, when set: the picker and the current release read through
    /// them (metadata-first, cached per connection) instead of `helm`.
    native: Option<Arc<dyn ReleaseSource>>,
    progress: ReleaseProgress,
    who: Arc<str>,
}

impl std::fmt::Debug for UpgradeFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpgradeFlow").finish_non_exhaustive()
    }
}

impl UpgradeFlow {
    /// A flow dispatching on `bus` as `who` (the local user, for the audit log), reading through
    /// `helm` until [`with_release_source`](Self::with_release_source) gives it the sessions'
    /// native readers, and following upgrades on `progress` (the handler's, see
    /// [`register_commands`](super::register_commands)).
    pub fn new(
        bus: CommandBus,
        helm: Arc<dyn HelmPort>,
        progress: ReleaseProgress,
        who: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            bus,
            helm,
            native: None,
            progress,
            who: who.into(),
        }
    }

    /// Reads releases through `source`'s native reader of each connection (the one the Releases
    /// page uses, with its decode cache), listing the namespaces the cluster's tab watches,
    /// instead of `helm list -A`, `helm history` and `helm get values`. `helm` then only renders
    /// and upgrades (through the guard).
    #[must_use]
    pub fn with_release_source(mut self, source: Arc<dyn ReleaseSource>) -> Self {
        self.native = Some(source);
        self
    }

    /// The reader for `cluster`'s releases.
    fn reader(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn HelmPort>> {
        match &self.native {
            Some(source) => source
                .helm(cluster)
                .ok_or_else(|| OxiError::network(format!("cluster {cluster} is not connected"))),
            None => Ok(self.helm.clone()),
        }
    }

    /// The namespaces the picker lists: the tab's with native reads, every one through `helm`.
    fn picker_scope(&self, cluster: &ClusterId) -> NamespaceSelection {
        self.native
            .as_ref()
            .map_or(NamespaceSelection::All, |source| source.namespaces(cluster))
    }

    /// The cluster's releases (latest revision each), by namespace then name, for the picker.
    /// With a [release source](Self::with_release_source), the namespaces the cluster's tab
    /// watches; otherwise all of them.
    ///
    /// # Errors
    ///
    /// The read's error (forbidden, not connected).
    pub async fn releases(&self, cluster: &ClusterId) -> OxiResult<Vec<Release>> {
        let reader = self.reader(cluster)?;
        let scope = self.picker_scope(cluster);
        let mut found = reader.list_releases(cluster, &scope).await?;
        found.sort_by(|a, b| {
            (&a.release.namespace, &a.release.name).cmp(&(&b.release.namespace, &b.release.name))
        });
        Ok(found)
    }

    /// What `release` runs now and its user-supplied values.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown release, the read's error otherwise.
    pub async fn current(&self, release: &ReleaseRef) -> OxiResult<CurrentRelease> {
        let reader = self.reader(&release.cluster)?;
        let history = reader.history(release).await?;
        let latest = history
            .first()
            .ok_or_else(|| OxiError::not_found(format!("release {release} has no revisions")))?;
        let values = reader.values(release, None, false).await?;
        let text = if values.is_empty() {
            String::new()
        } else {
            values.to_yaml()?
        };
        Ok(CurrentRelease {
            release: release.clone(),
            chart: latest.chart.clone(),
            app_version: latest.app_version.clone(),
            revision: latest.revision,
            status: latest.status,
            values: ValuesYaml::new(text),
        })
    }

    /// The preview of `spec`: a dry-run `helm::Upgrade` through the guard (the rendered chart, the
    /// server's verdict, the objects that change). Nothing is upgraded. The guard's confirmation
    /// of the rehearsal itself is answered here: the user asked for it and it changes nothing.
    ///
    /// # Errors
    ///
    /// [`UpgradeError::Invalid`] when the values are not a YAML map (nothing sent),
    /// [`UpgradeError::ReadOnly`] on a read-only cluster, [`UpgradeError::Failed`] when the chart
    /// does not render.
    pub async fn preview(&self, spec: UpgradeSpec) -> Result<UpgradePreview, UpgradeError> {
        spec.values
            .parse()
            .map_err(|error| UpgradeError::Invalid(error.to_string()))?;
        let context = self.context(spec.cluster(), true);
        let output = rehearse(&self.bus, spec.command(), context).await?;
        let data = output.data.unwrap_or(Value::Null);
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
        let manifest: Arc<str> = text("manifest").unwrap_or_default().into();
        Ok(UpgradePreview {
            summary: ManifestSummary::of(&manifest),
            changes: changes_of(&data),
            from_chart: text("from_chart").unwrap_or_default(),
            server_error: text("server_error"),
            manifest,
            spec,
        })
    }

    /// Asks the guard to upgrade what `preview` showed: the answer is its confirmation request
    /// (tier simple, risk medium), for the tab to show. Nothing runs until
    /// [`start`](Self::start) answers it, or [`decline`](Self::decline) drops it.
    ///
    /// # Errors
    ///
    /// [`UpgradeError::Invalid`] when the preview's server dry run refused the upgrade,
    /// [`UpgradeError::ReadOnly`] on a read-only cluster (audited as denied).
    pub async fn ask(&self, preview: &UpgradePreview) -> Result<ConfirmationRequest, UpgradeError> {
        if let Some(why) = &preview.server_error {
            return Err(UpgradeError::Invalid(why.clone()));
        }
        let context = self.context(preview.spec.cluster(), false);
        match self.bus.dispatch(preview.spec.command(), context).await {
            Ok(Outcome::NeedsConfirmation(request)) => Ok(request),
            Ok(Outcome::Completed(_)) => Err(UpgradeError::Failed(
                "the upgrade ran without a confirmation".to_owned(),
            )),
            Err(error) => Err(classify(error).into()),
        }
    }

    /// Upgrades with `preview`'s spec, answering `confirmation`: follows the release first, then
    /// dispatches with the answer. The result's `run` is the same shape as an install's.
    pub fn start(
        &self,
        preview: &UpgradePreview,
        confirmation: &ConfirmationRequest,
    ) -> UpgradeStart {
        let spec = preview.spec.clone();
        let lines = self.progress.subscribe(&spec.release);
        let bus = self.bus.clone();
        let progress = self.progress.clone();
        let context = self
            .context(spec.cluster(), false)
            .with_confirmation(Confirmation::simple(confirmation.token));
        let run = async move {
            let result = bus.dispatch(spec.command(), context).await;
            progress.forget(&spec.release);
            match result {
                Ok(Outcome::Completed(output)) => Ok(output.message.unwrap_or_default()),
                Ok(Outcome::NeedsConfirmation(_)) => Err(UpgradeError::Unconfirmed),
                Err(error) => Err(classify(error).into()),
            }
        };
        UpgradeStart {
            lines,
            run: run.boxed(),
        }
    }

    /// Declines `confirmation`: nothing is upgraded and the guard audits it as cancelled.
    pub async fn decline(&self, confirmation: ConfirmationRequest) {
        if let Err(error) = self.bus.decline(confirmation.token).await {
            tracing::debug!(%error, "declining an upgrade confirmation");
        }
    }

    fn context(&self, cluster: &ClusterId, dry_run: bool) -> DispatchContext {
        DispatchContext::new(Initiator::Ui, self.who.clone())
            .with_cluster(cluster.clone())
            .with_dry_run(dry_run)
    }
}

/// An upgrade that was confirmed: its output as it arrives and its end. Dropping `run` cancels the
/// upgrade (`helm` is killed; the guard audits it as cancelled).
pub struct UpgradeStart {
    /// Every line of `helm`'s output, in order; ends when the upgrade ends.
    pub lines: futures::channel::mpsc::UnboundedReceiver<oxikube_domain::helm::OutputLine>,
    /// The dispatch: the handler's message when the release was upgraded, else why not.
    pub run: futures::future::BoxFuture<'static, Result<String, UpgradeError>>,
}

/// The objects the preview's data names, rebuilt as keys (kind and name; the namespace is part of
/// the name text).
fn changes_of(data: &Value) -> ResourceChanges {
    use oxikube_domain::helm::ObjectKey;
    let keys = |field: &str| -> Vec<ObjectKey> {
        data.get("changes")
            .and_then(|c| c.get(field))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|text| {
                let (kind, rest) = text.split_once(' ').unwrap_or(("", text));
                let (namespace, name) = match rest.split_once('/') {
                    Some((ns, name)) => (Some(ns.to_owned()), name),
                    None => (None, rest),
                };
                ObjectKey {
                    kind: kind.to_owned(),
                    name: name.to_owned(),
                    namespace,
                }
            })
            .collect()
    };
    ResourceChanges {
        added: keys("added"),
        removed: keys("removed"),
        changed: keys("changed"),
        unchanged: data
            .get("changes")
            .and_then(|c| c.get("unchanged"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
    }
}
