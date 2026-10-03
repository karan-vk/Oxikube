//! Cluster-source fakes: [`FakeClusterSourcePort`] and [`FakeCloudDiscoveryPort`].

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    CloudDiscoveryPort, CloudProvider, CloudToolStatus, ClusterContext, ClusterSource,
    ClusterSourcePort, DiscoveredCluster, SourcesChanged,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

// --- ClusterSourcePort -------------------------------------------------------------------

/// Queued responses for each [`FakeClusterSourcePort`] method.
#[derive(Debug, Default)]
pub struct ClusterSourceScripts {
    /// `sources`.
    pub sources: Script<Vec<ClusterSource>>,
    /// `contexts`.
    pub contexts: Script<Vec<ClusterContext>>,
    /// `reload`.
    pub reload: Script<SourcesChanged>,
}

/// One call made on a [`FakeClusterSourcePort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterSourceCall {
    /// `sources()`.
    Sources,
    /// `contexts()`.
    Contexts,
    /// `subscribe()`.
    Subscribe,
    /// `reload()`.
    Reload,
}

#[derive(Default)]
struct SourceState {
    sources: Vec<ClusterSource>,
    contexts: Vec<ClusterContext>,
    subscribers: Vec<mpsc::UnboundedSender<SourcesChanged>>,
}

/// Fake `ClusterSourcePort`.
///
/// Fallbacks: `sources` and `contexts` return the configured lists, `reload` reports no
/// change. [`set_contexts`](Self::set_contexts) replaces the contexts and pushes the diff
/// to every `subscribe` stream; a non-empty scripted `reload` result is pushed the same
/// way, as a real source would after re-reading its files.
#[derive(Default)]
pub struct FakeClusterSourcePort {
    script: ClusterSourceScripts,
    calls: CallLog<ClusterSourceCall>,
    state: Mutex<SourceState>,
}

fake_plumbing!(
    FakeClusterSourcePort,
    ClusterSourceScripts,
    ClusterSourceCall
);

impl std::fmt::Debug for FakeClusterSourcePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("FakeClusterSourcePort")
            .field("sources", &state.sources.len())
            .field("contexts", &state.contexts.len())
            .field("subscribers", &state.subscribers.len())
            .finish_non_exhaustive()
    }
}

impl FakeClusterSourcePort {
    /// A fake with no sources and no contexts.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the configured sources.
    #[must_use]
    pub fn with_sources(self, sources: impl IntoIterator<Item = ClusterSource>) -> Self {
        self.state.lock().sources = sources.into_iter().collect();
        self
    }

    /// Sets the configured contexts (without notifying subscribers).
    #[must_use]
    pub fn with_contexts(self, contexts: impl IntoIterator<Item = ClusterContext>) -> Self {
        self.state.lock().contexts = contexts.into_iter().collect();
        self
    }

    /// Replaces the contexts, pushes the diff to subscribers when it is not empty, and
    /// returns it.
    pub fn set_contexts(
        &self,
        contexts: impl IntoIterator<Item = ClusterContext>,
    ) -> SourcesChanged {
        let mut state = self.state.lock();
        let new: Vec<_> = contexts.into_iter().collect();
        let diff = SourcesChanged::diff(&state.contexts, &new);
        state.contexts = new;
        Self::broadcast(&mut state, &diff);
        diff
    }

    /// Number of `subscribe` streams still alive.
    pub fn subscriber_count(&self) -> usize {
        let mut state = self.state.lock();
        state.subscribers.retain(|s| !s.is_closed());
        state.subscribers.len()
    }

    fn broadcast(state: &mut SourceState, diff: &SourcesChanged) {
        if diff.is_empty() {
            return;
        }
        state
            .subscribers
            .retain(|s| s.unbounded_send(diff.clone()).is_ok());
    }
}

#[async_trait]
impl ClusterSourcePort for FakeClusterSourcePort {
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>> {
        self.calls.record(ClusterSourceCall::Sources);
        self.script
            .sources
            .next_or_else(|| Ok(self.state.lock().sources.clone()))
    }

    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>> {
        self.calls.record(ClusterSourceCall::Contexts);
        self.script
            .contexts
            .next_or_else(|| Ok(self.state.lock().contexts.clone()))
    }

    fn subscribe(&self) -> BoxStream<'static, SourcesChanged> {
        self.calls.record(ClusterSourceCall::Subscribe);
        let (tx, rx) = mpsc::unbounded();
        self.state.lock().subscribers.push(tx);
        rx.boxed()
    }

    async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.calls.record(ClusterSourceCall::Reload);
        let changed = self
            .script
            .reload
            .next_or_else(|| Ok(SourcesChanged::default()))?;
        Self::broadcast(&mut self.state.lock(), &changed);
        Ok(changed)
    }
}

// --- CloudDiscoveryPort ------------------------------------------------------------------

/// Queued responses for each [`FakeCloudDiscoveryPort`] method.
#[derive(Debug, Default)]
pub struct CloudScripts {
    /// `tool_status`.
    pub tool_status: Script<CloudToolStatus>,
    /// `discover`.
    pub discover: Script<Vec<DiscoveredCluster>>,
}

/// One call made on a [`FakeCloudDiscoveryPort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudCall {
    /// `tool_status(provider)`.
    ToolStatus(CloudProvider),
    /// `discover(provider)`.
    Discover(CloudProvider),
}

/// Fake `CloudDiscoveryPort`.
///
/// Fallbacks: `tool_status` returns the configured status for the provider
/// (`NotInstalled` by default); `discover` returns the configured clusters of that
/// provider (none by default).
#[derive(Debug, Default)]
pub struct FakeCloudDiscoveryPort {
    script: CloudScripts,
    calls: CallLog<CloudCall>,
    status: Mutex<HashMap<CloudProvider, CloudToolStatus>>,
    clusters: Mutex<Vec<DiscoveredCluster>>,
}

fake_plumbing!(FakeCloudDiscoveryPort, CloudScripts, CloudCall);

impl FakeCloudDiscoveryPort {
    /// A fake where every CLI is missing and nothing is discovered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the status reported for `provider`.
    #[must_use]
    pub fn with_status(self, provider: CloudProvider, status: CloudToolStatus) -> Self {
        self.status.lock().insert(provider, status);
        self
    }

    /// Sets the clusters `discover` returns (filtered by provider).
    #[must_use]
    pub fn with_clusters(self, clusters: impl IntoIterator<Item = DiscoveredCluster>) -> Self {
        *self.clusters.lock() = clusters.into_iter().collect();
        self
    }
}

#[async_trait]
impl CloudDiscoveryPort for FakeCloudDiscoveryPort {
    async fn tool_status(&self, provider: CloudProvider) -> OxiResult<CloudToolStatus> {
        self.calls.record(CloudCall::ToolStatus(provider));
        self.script.tool_status.next_or_else(|| {
            Ok(self
                .status
                .lock()
                .get(&provider)
                .copied()
                .unwrap_or(CloudToolStatus::NotInstalled))
        })
    }

    async fn discover(&self, provider: CloudProvider) -> OxiResult<Vec<DiscoveredCluster>> {
        self.calls.record(CloudCall::Discover(provider));
        self.script.discover.next_or_else(|| {
            Ok(self
                .clusters
                .lock()
                .iter()
                .filter(|c| c.provider == provider)
                .cloned()
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use futures::executor::block_on;
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_domain::{ErrorKind, OxiError};
    use oxikube_ports::SourceId;

    fn ctx(name: &str) -> ClusterContext {
        let context = ContextName::new(name);
        ClusterContext {
            cluster: ClusterId::new("/home/u/.kube/config", &context),
            context,
            source: SourceId("kubeconfig".into()),
            server: Some("https://127.0.0.1:6443".into()),
            default_namespace: None,
        }
    }

    #[test]
    fn cluster_source_scripts_records_and_notifies_subscribers() {
        let fake = FakeClusterSourcePort::new().with_contexts([ctx("kind-a")]);
        fake.script()
            .contexts
            .push_ok(Vec::new())
            .push_err(OxiError::validation("bad kubeconfig"));
        assert!(block_on(fake.contexts()).unwrap().is_empty());
        assert_eq!(
            block_on(fake.contexts()).unwrap_err().kind(),
            ErrorKind::Validation
        );
        assert_eq!(block_on(fake.contexts()).unwrap(), vec![ctx("kind-a")]);
        assert!(block_on(fake.sources()).unwrap().is_empty());

        let mut changes = fake.subscribe();
        assert_eq!(fake.subscriber_count(), 1);
        let diff = fake.set_contexts([ctx("kind-a"), ctx("kind-b")]);
        assert_eq!(diff.added, vec![ctx("kind-b")]);
        assert_eq!(block_on(changes.next()), Some(diff));
        assert!(block_on(fake.reload()).unwrap().is_empty());
        assert!(
            changes.next().now_or_never().is_none(),
            "empty reload is not pushed"
        );

        let removed = SourcesChanged {
            removed: vec![ctx("kind-b").cluster],
            ..SourcesChanged::default()
        };
        fake.script().reload.push_ok(removed.clone());
        assert_eq!(block_on(fake.reload()).unwrap(), removed);
        assert_eq!(block_on(changes.next()), Some(removed));
        drop(changes);
        assert_eq!(fake.subscriber_count(), 0);

        assert_eq!(
            fake.recorded_calls(),
            vec![
                ClusterSourceCall::Contexts,
                ClusterSourceCall::Contexts,
                ClusterSourceCall::Contexts,
                ClusterSourceCall::Sources,
                ClusterSourceCall::Subscribe,
                ClusterSourceCall::Reload,
                ClusterSourceCall::Reload,
            ]
        );
    }

    #[test]
    fn cloud_discovery_scripts_and_falls_back_per_provider() {
        let eks = DiscoveredCluster {
            provider: CloudProvider::Aws,
            name: "prod".into(),
            region: Some("eu-west-1".into()),
            account: Some("000000000000".into()),
            endpoint: None,
        };
        let fake = FakeCloudDiscoveryPort::new()
            .with_status(CloudProvider::Aws, CloudToolStatus::Ready)
            .with_clusters([eks.clone()]);
        fake.script()
            .tool_status
            .push_ok(CloudToolStatus::NotAuthenticated)
            .push_err(OxiError::timeout("aws cli hung"));
        assert_eq!(
            block_on(fake.tool_status(CloudProvider::Aws)).unwrap(),
            CloudToolStatus::NotAuthenticated
        );
        assert_eq!(
            block_on(fake.tool_status(CloudProvider::Aws))
                .unwrap_err()
                .kind(),
            ErrorKind::Timeout
        );
        assert_eq!(
            block_on(fake.tool_status(CloudProvider::Aws)).unwrap(),
            CloudToolStatus::Ready
        );
        assert_eq!(
            block_on(fake.tool_status(CloudProvider::Gcp)).unwrap(),
            CloudToolStatus::NotInstalled
        );
        assert_eq!(
            block_on(fake.discover(CloudProvider::Aws)).unwrap(),
            vec![eks]
        );
        assert!(
            block_on(fake.discover(CloudProvider::Azure))
                .unwrap()
                .is_empty()
        );
        assert_eq!(fake.recorded_calls().len(), 6);
        assert_eq!(
            fake.recorded_calls()[5],
            CloudCall::Discover(CloudProvider::Azure)
        );
    }
}
