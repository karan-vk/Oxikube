//! The cached charts catalog: every chart of every repository, read with one
//! `helm search repo --versions` per catalog generation and filtered in memory per keyword.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use oxikube_domain::helm::{ChartSummary, SearchQuery, group_hits};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::HelmPort;

/// Every chart, with a lower-cased text to match keywords against (built once, not per
/// keystroke).
pub(super) struct Catalog {
    charts: Arc<Vec<ChartSummary>>,
    /// Per chart: `repo/name` and its distinct descriptions, lower-cased.
    haystacks: Vec<String>,
}

impl Catalog {
    fn new(charts: Vec<ChartSummary>) -> Self {
        let haystacks = charts
            .iter()
            .map(|chart| {
                let mut text = chart.reference();
                let mut seen: Vec<&str> = Vec::new();
                for version in &chart.versions {
                    if !seen.contains(&version.description.as_str()) {
                        seen.push(&version.description);
                        text.push('\n');
                        text.push_str(&version.description);
                    }
                }
                text.to_lowercase()
            })
            .collect();
        Self {
            charts: Arc::new(charts),
            haystacks,
        }
    }

    /// Every chart.
    pub(super) fn all(&self) -> Arc<Vec<ChartSummary>> {
        self.charts.clone()
    }

    /// The charts whose `repo/name` or a description contains the lower-cased `key`, like Helm's
    /// literal search (Chart.yaml keywords are not in `helm search -o json`, so they do not
    /// match).
    pub(super) fn matching(&self, key: &str) -> Arc<Vec<ChartSummary>> {
        Arc::new(
            self.charts
                .iter()
                .zip(&self.haystacks)
                .filter(|(_, haystack)| haystack.contains(key))
                .map(|(chart, _)| chart.clone())
                .collect(),
        )
    }
}

/// A failure that can be handed to every waiter of one load.
#[derive(Clone)]
pub(super) struct LoadError {
    kind: ErrorKind,
    message: String,
    retryable: bool,
}

impl From<LoadError> for OxiError {
    fn from(error: LoadError) -> Self {
        OxiError::new(error.kind, error.message).with_retryable(error.retryable)
    }
}

/// One load of the catalog, shared by every search that waits for it. A search that is dropped
/// (the user typed again) leaves the load in the cache, and the next search resumes it instead of
/// starting `helm` over.
pub(super) type Loading = Shared<BoxFuture<'static, Result<Arc<Catalog>, LoadError>>>;

/// Starts a load over `helm` (it runs as the waiters poll it).
pub(super) fn load(helm: Arc<dyn HelmPort>) -> Loading {
    async move {
        let hits: OxiResult<_> = helm
            .search(&SearchQuery {
                keyword: String::new(),
                all_versions: true,
            })
            .await;
        hits.map(|hits| Arc::new(Catalog::new(group_hits(hits))))
            .map_err(|error| LoadError {
                kind: error.kind(),
                message: error.message().to_owned(),
                retryable: error.is_retryable(),
            })
    }
    .boxed()
    .shared()
}
