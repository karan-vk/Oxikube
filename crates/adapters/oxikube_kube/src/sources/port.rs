//! [`KubeconfigSources`] as a [`ClusterSourcePort`].

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    ClusterContext, ClusterSource, ClusterSourcePort, SourceDiagnostic, SourceStatus,
    SourcesChanged, UserSource, UserSourceKind,
};

use super::{KubeconfigSources, pasted};

#[async_trait]
impl ClusterSourcePort for KubeconfigSources {
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>> {
        Ok(self.inner.snapshot().await?.sources.clone())
    }

    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>> {
        Ok(self.inner.snapshot().await?.contexts())
    }

    fn subscribe(&self) -> BoxStream<'static, SourcesChanged> {
        let (tx, rx) = mpsc::unbounded();
        self.inner.subscribers.lock().push(tx);
        rx.boxed()
    }

    async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.inner.reload().await
    }

    async fn set_user_sources(&self, sources: &[UserSource]) -> OxiResult<SourcesChanged> {
        {
            let mut config = self.inner.config.write();
            config.include_default = sources.iter().any(|s| s.kind == UserSourceKind::Default);
            config.extra_paths = sources
                .iter()
                .filter(|s| s.kind != UserSourceKind::Default)
                .filter_map(|s| s.path.clone())
                .collect();
        }
        let diff = self.inner.reload().await;
        // The watcher registers directories by what the list says; tell it to look again, even
        // when the reload failed (the list did change).
        self.inner
            .rewatch
            .send_modify(|generation| *generation += 1);
        diff
    }

    async fn source_statuses(&self) -> OxiResult<Vec<SourceStatus>> {
        Ok(self.inner.snapshot().await?.statuses.clone())
    }

    async fn source_diagnostics(&self) -> OxiResult<Vec<SourceDiagnostic>> {
        // Load first when nothing was read yet, as `source_statuses` does.
        self.inner.snapshot().await?;
        Ok(self.inner.port_diagnostics())
    }

    fn subscribe_diagnostics(&self) -> BoxStream<'static, Vec<SourceDiagnostic>> {
        self.inner.subscribe_diagnostics()
    }

    async fn validate_kubeconfig(&self, text: &str) -> OxiResult<usize> {
        pasted::parse(text).map(|config| config.contexts.len())
    }
}
