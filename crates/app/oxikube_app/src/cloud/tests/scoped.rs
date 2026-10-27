//! [`ScopedPort`]: a provider whose listings fail or hang per scope, to test fan-out and
//! cancellation. Everything else is the testkit fake's.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::channel::mpsc;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, DiscoveredCluster,
    KubeconfigEntry,
};
use oxikube_testkit::FakeCloudDiscoveryPort;
use parking_lot::Mutex;

/// Counts a listing future dropped before it finished (a killed CLI process).
struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) struct ScopedPort {
    inner: FakeCloudDiscoveryPort,
    failing: HashSet<CloudScope>,
    hanging: HashSet<CloudScope>,
    /// One message per listing that started.
    started: Mutex<Option<mpsc::UnboundedSender<CloudScope>>>,
    /// Hanging listings dropped by a cancel.
    pub dropped: Arc<AtomicUsize>,
}

impl ScopedPort {
    pub fn new(inner: FakeCloudDiscoveryPort) -> Self {
        Self {
            inner,
            failing: HashSet::new(),
            hanging: HashSet::new(),
            started: Mutex::new(None),
            dropped: Arc::default(),
        }
    }

    /// Listing `scope` fails with an `Auth` error.
    #[must_use]
    pub fn failing(mut self, scope: CloudScope) -> Self {
        self.failing.insert(scope);
        self
    }

    /// Listing `scope` never finishes.
    #[must_use]
    pub fn hanging(mut self, scope: CloudScope) -> Self {
        self.hanging.insert(scope);
        self
    }

    /// A stream of the scopes whose listing started.
    pub fn started(&self) -> mpsc::UnboundedReceiver<CloudScope> {
        let (tx, rx) = mpsc::unbounded();
        *self.started.lock() = Some(tx);
        rx
    }
}

#[async_trait]
impl CloudDiscoveryPort for ScopedPort {
    fn id(&self) -> CloudProvider {
        self.inner.id()
    }

    async fn detect(&self) -> OxiResult<CloudDetection> {
        self.inner.detect().await
    }

    async fn scopes(&self) -> OxiResult<Vec<CloudScope>> {
        self.inner.scopes().await
    }

    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        if let Some(tx) = self.started.lock().as_ref() {
            let _ = tx.unbounded_send(scope.clone());
        }
        if self.hanging.contains(scope) {
            let _counter = DropCounter(self.dropped.clone());
            std::future::pending::<()>().await;
        }
        if self.failing.contains(scope) {
            return Err(OxiError::auth(
                "the SSO session of this profile has expired: run `aws sso login --profile dev`",
                false,
            ));
        }
        self.inner.list_clusters(scope).await
    }

    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
        self.inner.kubeconfig_entry(cluster).await
    }
}

impl std::fmt::Debug for ScopedPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedPort").finish_non_exhaustive()
    }
}

/// `port` as the service takes it, keeping the concrete handle.
pub(super) fn shared(port: ScopedPort) -> (Arc<ScopedPort>, Vec<Arc<dyn CloudDiscoveryPort>>) {
    let port = Arc::new(port);
    let providers: Vec<Arc<dyn CloudDiscoveryPort>> = vec![port.clone()];
    (port, providers)
}
