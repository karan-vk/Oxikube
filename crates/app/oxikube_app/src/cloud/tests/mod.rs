//! Tests of cloud discovery over `oxikube_testkit` fakes: fake providers, an in-memory state db
//! and filesystem, a fake cluster source and clock. Runs are spawned on the test's Tokio
//! runtime; nothing touches the disk or runs a CLI.

mod cache;
mod managed;
mod refresh;
mod scoped;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_ports::{
    CloudDiscoveryPort, CloudProvider, CloudScope, ClusterSource, DiscoveredCluster, SourceId,
    SourceKind, UserSource,
};
use oxikube_testkit::{
    FakeClockPort, FakeCloudDiscoveryPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    FakeFsPort, FakeStatePort,
};

use super::{CloudDiscoveryEvent, CloudDiscoveryService, DiscoveryDeps};
use crate::session::ClusterSessionManager;
use crate::sources::{KubeconfigSourcesService, MemorySourceList};
use crate::store::Spawner;

/// The kubeconfigs directory of the test config dir.
pub(super) const DIR: &str = "/config/kubeconfigs";

pub(super) fn aws_scope(profile: &str, region: &str) -> CloudScope {
    CloudScope::Aws {
        profile: profile.into(),
        region: region.into(),
    }
}

/// An EKS cluster named `name` in `scope`, with an endpoint and a public CA.
pub(super) fn eks(scope: &CloudScope, name: &str) -> DiscoveredCluster {
    let CloudScope::Aws { region, .. } = scope else {
        panic!("an AWS scope");
    };
    DiscoveredCluster {
        endpoint: Some(format!("https://{name}.gr7.{region}.eks.amazonaws.com")),
        ca_data: Some("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t".into()),
        ..DiscoveredCluster::new(scope.clone(), name, region.clone())
    }
}

/// The service and the fakes behind it.
pub(super) struct Fixture {
    pub state: Arc<FakeStatePort>,
    pub fs: Arc<FakeFsPort>,
    pub source: Arc<FakeClusterSourcePort>,
    pub list: Arc<MemorySourceList>,
    pub clock: Arc<FakeClockPort>,
    pub sessions: ClusterSessionManager,
    pub service: CloudDiscoveryService,
}

impl Fixture {
    /// A service over `providers`, with an empty cache and filesystem and the default source.
    pub fn new(providers: Vec<Arc<dyn CloudDiscoveryPort>>) -> Self {
        Self::with(providers, FakeFsPort::new(), FakeClusterSourcePort::new())
    }

    /// As [`new`](Self::new), over the given filesystem and cluster source.
    pub fn with(
        providers: Vec<Arc<dyn CloudDiscoveryPort>>,
        fs: FakeFsPort,
        source: FakeClusterSourcePort,
    ) -> Self {
        let state = Arc::new(FakeStatePort::new());
        let fs = Arc::new(fs);
        let source = Arc::new(source);
        let list = Arc::new(MemorySourceList::new([UserSource::default_source()]));
        let clock = Arc::new(FakeClockPort::new(
            Timestamp::from_second(1_800_000_000).unwrap(),
        ));
        let sessions = ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            source.clone(),
            clock.clone(),
        );
        let sources = KubeconfigSourcesService::new(
            source.clone(),
            fs.clone(),
            list.clone(),
            PathBuf::from(DIR),
        );
        let spawner: Arc<dyn Spawner> = Arc::new(|task| {
            tokio::spawn(task);
        });
        let service = CloudDiscoveryService::new(DiscoveryDeps {
            providers,
            state: state.clone(),
            fs: fs.clone(),
            sources,
            source: source.clone(),
            sessions: sessions.clone(),
            clock: clock.clone(),
            spawner,
        });
        Self {
            state,
            fs,
            source,
            list,
            clock,
            sessions,
            service,
        }
    }

    /// One refresh, run to its end.
    pub async fn refresh(
        &self,
        provider: CloudProvider,
        mode: super::RefreshMode,
    ) -> Vec<CloudDiscoveryEvent> {
        collect(self.service.refresh(provider, None, mode)).await
    }

    /// Moves the clock on.
    pub fn advance(&self, by: Duration) {
        self.clock.advance(by);
    }

    /// The managed file of `provider`, as text.
    pub fn managed_text(&self, provider: CloudProvider) -> Option<String> {
        self.fs
            .file(self.service.managed_path(provider))
            .map(|bytes| String::from_utf8(bytes).unwrap())
    }
}

/// Every event of `events`, with a deadline so a run that never ends fails the test.
pub(super) async fn collect(
    events: BoxStream<'static, CloudDiscoveryEvent>,
) -> Vec<CloudDiscoveryEvent> {
    tokio::time::timeout(Duration::from_secs(10), events.collect())
        .await
        .expect("the run ends")
}

/// A short name for each event, to compare sequences.
pub(super) fn names(events: &[CloudDiscoveryEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            CloudDiscoveryEvent::Cached { stale: false, .. } => "cached-fresh",
            CloudDiscoveryEvent::Cached { stale: true, .. } => "cached-stale",
            CloudDiscoveryEvent::Detected(_) => "detected",
            CloudDiscoveryEvent::Started { .. } => "started",
            CloudDiscoveryEvent::ScopeDone { .. } => "scope-done",
            CloudDiscoveryEvent::ScopeFailed { .. } => "scope-failed",
            CloudDiscoveryEvent::Finished { .. } => "finished",
            CloudDiscoveryEvent::Failed { .. } => "failed",
            CloudDiscoveryEvent::Cancelled { .. } => "cancelled",
        })
        .collect()
}

/// The clusters of the terminal `Finished` event.
pub(super) fn finished(events: &[CloudDiscoveryEvent]) -> (Vec<DiscoveredCluster>, usize) {
    match events.last() {
        Some(CloudDiscoveryEvent::Finished {
            clusters,
            failed_scopes,
            ..
        }) => (clusters.clone(), *failed_scopes),
        other => panic!("the run did not finish: {other:?}"),
    }
}

/// A ready AWS fake with `scopes` and `clusters`.
pub(super) fn aws(
    scopes: impl IntoIterator<Item = CloudScope>,
    clusters: impl IntoIterator<Item = DiscoveredCluster>,
) -> Arc<FakeCloudDiscoveryPort> {
    Arc::new(
        FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
            .with_scopes(scopes)
            .with_clusters(clusters),
    )
}

/// A cluster source that lists the managed AWS file as a source, so a context read from it can
/// be found.
pub(super) fn source_with_managed(
    contexts: Vec<oxikube_ports::ClusterContext>,
) -> FakeClusterSourcePort {
    FakeClusterSourcePort::new()
        .with_sources([ClusterSource {
            id: SourceId(format!("file:{DIR}/aws.yaml")),
            kind: SourceKind::KubeconfigFile,
            label: format!("{DIR}/aws.yaml"),
            path: Some(PathBuf::from(DIR).join("aws.yaml")),
        }])
        .with_contexts(contexts)
}
