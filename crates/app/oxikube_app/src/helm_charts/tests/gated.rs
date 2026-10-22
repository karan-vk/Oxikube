//! A `HelmPort` whose `search` (and, when asked, `repos`) waits until released, to see how many
//! loads start and what happens around them.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{
    ChartHit, ChartRef, HelmCapabilities, InstallRequest, Release, ReleaseRef, RepoEntry, Revision,
    RollbackRequest, SearchQuery, ShowKind, UninstallRequest, UpgradeRequest, Values,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{HelmOutput, HelmPort};
use oxikube_testkit::FakeHelmPort;
use tokio::sync::Semaphore;

/// Delegates to `inner`; `search` counts itself, then waits for a [`release`](Self::release).
pub(super) struct Gated {
    pub inner: Arc<FakeHelmPort>,
    pub started: AtomicUsize,
    gate: Semaphore,
    /// When set, `repos` waits for a permit of its own.
    repos_gate: Option<Semaphore>,
}

impl Gated {
    pub fn new(inner: Arc<FakeHelmPort>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            started: AtomicUsize::new(0),
            gate: Semaphore::new(0),
            repos_gate: None,
        })
    }

    /// Like [`new`](Self::new), with `repos` held too.
    pub fn holding_repos(inner: Arc<FakeHelmPort>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            started: AtomicUsize::new(0),
            gate: Semaphore::new(0),
            repos_gate: Some(Semaphore::new(0)),
        })
    }

    /// Lets one waiting (or future) `repos` through.
    pub fn release_repos(&self) {
        if let Some(gate) = &self.repos_gate {
            gate.add_permits(1);
        }
    }

    /// Lets one waiting (or future) search through.
    pub fn release(&self) {
        self.gate.add_permits(1);
    }
}

#[async_trait]
impl HelmPort for Gated {
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<Release>> {
        self.inner.list_releases(cluster, namespaces).await
    }
    async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>> {
        self.inner.history(release).await
    }
    async fn values(&self, r: &ReleaseRef, rev: Option<u32>, all: bool) -> OxiResult<Values> {
        self.inner.values(r, rev, all).await
    }
    async fn manifest(&self, r: &ReleaseRef, rev: Option<u32>) -> OxiResult<String> {
        self.inner.manifest(r, rev).await
    }
    async fn notes(&self, r: &ReleaseRef, rev: Option<u32>) -> OxiResult<Option<String>> {
        self.inner.notes(r, rev).await
    }
    async fn repos(&self) -> OxiResult<Vec<RepoEntry>> {
        if let Some(gate) = &self.repos_gate {
            gate.acquire().await.expect("open").forget();
        }
        self.inner.repos().await
    }
    async fn search(&self, query: &SearchQuery) -> OxiResult<Vec<ChartHit>> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.gate.acquire().await.expect("open").forget();
        self.inner.search(query).await
    }
    async fn show(&self, chart: &ChartRef, kind: ShowKind) -> OxiResult<String> {
        self.inner.show(chart, kind).await
    }
    async fn template(&self, request: &InstallRequest) -> OxiResult<String> {
        self.inner.template(request).await
    }
    async fn install(&self, request: &InstallRequest) -> OxiResult<HelmOutput> {
        self.inner.install(request).await
    }
    async fn upgrade(&self, request: &UpgradeRequest) -> OxiResult<HelmOutput> {
        self.inner.upgrade(request).await
    }
    async fn rollback(&self, request: &RollbackRequest) -> OxiResult<()> {
        self.inner.rollback(request).await
    }
    async fn uninstall(&self, request: &UninstallRequest) -> OxiResult<()> {
        self.inner.uninstall(request).await
    }
    fn capabilities(&self) -> HelmCapabilities {
        self.inner.capabilities()
    }
}
