//! [`InstallBackend`]: where the install tab gets its data and sends its installs.

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_app::guard::ConfirmationRequest;
use oxikube_app::{
    ChartsReadiness, ChartsService, InstallError, InstallFlow, InstallPreview, InstallSpec,
    InstallStart,
};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{ChartPart, ChartRef};
use oxikube_domain::ids::ClusterId;

/// The tab's view of the app. Every future runs on the runtime (`spawn_kube`), never on the UI
/// thread; nothing here waits.
pub trait InstallBackend: 'static {
    /// Whether `helm` is usable at all (installing needs the binary).
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>>;

    /// The chart's default `values.yaml` (`helm show values`), comments included.
    fn defaults(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Arc<str>>>;

    /// The chart's versions, newest first (the catalog's search).
    fn versions(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Vec<String>>>;

    /// The cluster's namespaces, sorted.
    fn namespaces(&self, cluster: ClusterId) -> BoxFuture<'static, OxiResult<Vec<String>>>;

    /// The preview: the guard's dry run of `spec`.
    fn preview(
        &self,
        spec: InstallSpec,
    ) -> BoxFuture<'static, Result<InstallPreview, InstallError>>;

    /// The guard's confirmation request for installing what `preview` showed.
    fn ask(
        &self,
        preview: InstallPreview,
    ) -> BoxFuture<'static, Result<ConfirmationRequest, InstallError>>;

    /// Installs, answering `confirmation`: the output lines and the dispatch (dropping it
    /// cancels).
    fn start(&self, preview: &InstallPreview, confirmation: &ConfirmationRequest) -> InstallStart;

    /// Declines `confirmation` (audited as cancelled).
    fn decline(&self, confirmation: ConfirmationRequest) -> BoxFuture<'static, ()>;
}

/// The backend over the app's [`InstallFlow`] and the charts catalog's [`ChartsService`] (which
/// caches the chart's values and versions the catalog already read).
#[derive(Clone)]
pub struct FlowBackend {
    flow: InstallFlow,
    charts: ChartsService,
}

impl FlowBackend {
    /// A backend over `flow` and `charts`.
    pub fn new(flow: InstallFlow, charts: ChartsService) -> Self {
        Self { flow, charts }
    }
}

impl InstallBackend for FlowBackend {
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>> {
        let charts = self.charts.clone();
        Box::pin(async move { charts.ready().await })
    }

    fn defaults(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Arc<str>>> {
        let charts = self.charts.clone();
        Box::pin(async move { charts.detail(&chart, ChartPart::Values).await })
    }

    fn versions(&self, chart: ChartRef) -> BoxFuture<'static, OxiResult<Vec<String>>> {
        let charts = self.charts.clone();
        Box::pin(async move {
            let found = charts.search(&chart.name).await?;
            Ok(found
                .iter()
                .find(|c| c.name == chart.name && c.repo == chart.repo)
                .map(|c| c.versions.iter().map(|v| v.version.clone()).collect())
                .unwrap_or_default())
        })
    }

    fn namespaces(&self, cluster: ClusterId) -> BoxFuture<'static, OxiResult<Vec<String>>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.namespaces(&cluster).await })
    }

    fn preview(
        &self,
        spec: InstallSpec,
    ) -> BoxFuture<'static, Result<InstallPreview, InstallError>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.preview(spec).await })
    }

    fn ask(
        &self,
        preview: InstallPreview,
    ) -> BoxFuture<'static, Result<ConfirmationRequest, InstallError>> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.ask(&preview).await })
    }

    fn start(&self, preview: &InstallPreview, confirmation: &ConfirmationRequest) -> InstallStart {
        self.flow.start(preview, confirmation)
    }

    fn decline(&self, confirmation: ConfirmationRequest) -> BoxFuture<'static, ()> {
        let flow = self.flow.clone();
        Box::pin(async move { flow.decline(confirmation).await })
    }
}
