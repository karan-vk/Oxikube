//! [`ChartsBackend`]: where the catalog page gets its data.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oxikube_app::{ChartsReadiness, ChartsService};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{ChartPart, ChartRef, ChartSummary};

/// The page's view of the app. Every method returns a future the page runs on the runtime with
/// `spawn_kube`, never on the UI thread; nothing here waits.
pub trait ChartsBackend: 'static {
    /// Whether the catalog can be used (no `helm`, no repositories, ready). The first call makes
    /// the adapter look for the binary.
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>>;

    /// The charts matching `keyword`, each with all its versions.
    fn search(&self, keyword: String) -> BoxFuture<'static, OxiResult<Arc<Vec<ChartSummary>>>>;

    /// `part` of `chart` (a chart version): the README, the default values, the metadata.
    fn detail(&self, chart: ChartRef, part: ChartPart) -> BoxFuture<'static, OxiResult<Arc<str>>>;

    /// One item whenever a repository changed and the catalog should be read again.
    fn changes(&self) -> BoxStream<'static, ()>;
}

/// The backend over the app's [`ChartsService`].
#[derive(Clone)]
pub struct ServiceBackend {
    service: ChartsService,
}

impl ServiceBackend {
    /// A backend over `service`.
    pub fn new(service: ChartsService) -> Self {
        Self { service }
    }
}

impl ChartsBackend for ServiceBackend {
    fn ready(&self) -> BoxFuture<'static, OxiResult<ChartsReadiness>> {
        let service = self.service.clone();
        Box::pin(async move { service.ready().await })
    }

    fn search(&self, keyword: String) -> BoxFuture<'static, OxiResult<Arc<Vec<ChartSummary>>>> {
        let service = self.service.clone();
        Box::pin(async move { service.search(&keyword).await })
    }

    fn detail(&self, chart: ChartRef, part: ChartPart) -> BoxFuture<'static, OxiResult<Arc<str>>> {
        let service = self.service.clone();
        Box::pin(async move { service.detail(&chart, part).await })
    }

    fn changes(&self) -> BoxStream<'static, ()> {
        futures::stream::unfold(self.service.catalog_changes(), |mut changes| async move {
            changes.changed().await.then_some(((), changes))
        })
        .boxed()
    }
}
