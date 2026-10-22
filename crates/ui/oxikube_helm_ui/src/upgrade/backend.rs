//! [`UpgradeBackend`]: where the upgrade tab gets its data and sends its upgrades.

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_app::guard::ConfirmationRequest;
use oxikube_app::{
    ChartsReadiness, ChartsService, CurrentRelease, UpgradeError, UpgradeFlow, UpgradePreview,
    UpgradeSpec, UpgradeStart,
};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{ChartPart, ChartRef, Release, ReleaseRef};
use oxikube_domain::ids::ClusterId;

/// A chart the configured repositories offer under the release's chart name, with its versions
/// (newest first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartSource {
    /// The chart in its repository (no version).
    pub chart: ChartRef,
    /// Its versions, newest first.
    pub versions: Vec<String>,
}

/// The tab's view of the app. Every future runs on the runtime (`spawn_kube`), never on the UI
/// thread; nothing here waits.
pub trait UpgradeBackend: 'static {
    /// Whether `helm` is usable at all (upgrading and the chart catalog need the binary; reading
    /// the release does not).
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>>;

    /// The cluster's releases, for the picker.
    fn releases(&self, cluster: ClusterId) -> BoxFuture<'static, OxiResult<Vec<Release>>>;

    /// What `release` runs now and its user-supplied values (read natively).
    fn current(&self, release: ReleaseRef) -> BoxFuture<'static, OxiResult<CurrentRelease>>;

    /// The charts named `name` in the configured repositories, with their versions.
    fn sources(&self, name: String) -> BoxFuture<'static, OxiResult<Vec<ChartSource>>>;

    /// The chart's default `values.yaml` at the chart's version (`helm show values`).
    fn defaults(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Arc<str>>>;

    /// The preview: the guard's dry run of `spec`.
    fn preview(
        &self,
        spec: UpgradeSpec,
    ) -> BoxFuture<'static, Result<UpgradePreview, UpgradeError>>;

    /// The guard's confirmation request for upgrading what `preview` showed.
    fn ask(
        &self,
        preview: UpgradePreview,
    ) -> BoxFuture<'static, Result<ConfirmationRequest, UpgradeError>>;

    /// Upgrades, answering `confirmation`: the output lines and the dispatch (dropping it
    /// cancels).
    fn start(&self, preview: &UpgradePreview, confirmation: &ConfirmationRequest) -> UpgradeStart;

    /// Declines `confirmation` (audited as cancelled).
    fn decline(&self, confirmation: ConfirmationRequest) -> BoxFuture<'static, ()>;
}

/// The backend over the app's [`UpgradeFlow`] and the charts catalog's [`ChartsService`] (which
/// caches the chart's values and versions the catalog already read).
#[derive(Clone)]
pub struct FlowBackend {
    flow: UpgradeFlow,
    charts: ChartsService,
}

impl FlowBackend {
    /// A backend over `flow` and `charts`.
    pub fn new(flow: UpgradeFlow, charts: ChartsService) -> Self {
        Self { flow, charts }
    }
}

impl UpgradeBackend for FlowBackend {
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>> {
        let charts = self.charts.clone();
        Box::pin(async move { charts.ready().await })
    }

    fn releases(&self, cluster: ClusterId) -> BoxFuture<'static, OxiResult<Vec<Release>>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.releases(&cluster).await })
    }

    fn current(&self, release: ReleaseRef) -> BoxFuture<'static, OxiResult<CurrentRelease>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.current(&release).await })
    }

    fn sources(&self, name: String) -> BoxFuture<'static, OxiResult<Vec<ChartSource>>> {
        let charts = self.charts.clone();
        Box::pin(async move {
            let found = charts.search(&name).await?;
            Ok(found
                .iter()
                .filter(|c| c.name == name)
                .filter_map(|c| {
                    Some(ChartSource {
                        chart: ChartRef::new(&c.name, "").in_repo(c.repo.clone()?),
                        versions: c.versions.iter().map(|v| v.version.clone()).collect(),
                    })
                })
                .collect())
        })
    }

    fn defaults(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Arc<str>>> {
        let charts = self.charts.clone();
        Box::pin(async move { charts.detail(&chart, ChartPart::Values).await })
    }

    fn preview(
        &self,
        spec: UpgradeSpec,
    ) -> BoxFuture<'static, Result<UpgradePreview, UpgradeError>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.preview(spec).await })
    }

    fn ask(
        &self,
        preview: UpgradePreview,
    ) -> BoxFuture<'static, Result<ConfirmationRequest, UpgradeError>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.ask(&preview).await })
    }

    fn start(&self, preview: &UpgradePreview, confirmation: &ConfirmationRequest) -> UpgradeStart {
        self.flow.start(preview, confirmation)
    }

    fn decline(&self, confirmation: ConfirmationRequest) -> BoxFuture<'static, ()> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.decline(confirmation).await })
    }
}
