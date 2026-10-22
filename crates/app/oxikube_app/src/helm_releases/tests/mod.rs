//! Tests of the releases service over the testkit fakes. Stores run on the test's Tokio runtime.

mod commands;
mod filter;
mod service;
mod watch;

use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{ChartRef, Release, ReleaseRef, ReleaseStatus, Values};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::HelmPort;
use oxikube_testkit::{FakeClockPort, FakeHelmPort, FakeResourcePort, FakeTableFeedPort};
use parking_lot::Mutex;
use serde_json::json;
use tokio::sync::broadcast;

use super::{ReleaseSource, ReleasesService};
use crate::helm::KindScopes;
use crate::session::SessionUpdate;
use crate::store::{ResourceStore, StoreOptions, StorePorts, StoreRuntime};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

pub(super) fn release_ref(name: &str) -> ReleaseRef {
    ReleaseRef::new(cluster(), "apps", name)
}

pub(super) fn release(name: &str, status: ReleaseStatus) -> Release {
    Release::summary(
        release_ref(name),
        ChartRef::new("web", "1.2.3"),
        2,
        status,
        Timestamp::from_second(1_700_000_000).unwrap(),
    )
}

pub(super) fn values(json: serde_json::Value) -> Values {
    Values::from_json(json)
}

/// A source over a scripted `FakeHelmPort`, a store on a fake resource port and a session
/// update channel the tests feed.
pub(super) struct Fixture {
    pub helm: Arc<FakeHelmPort>,
    pub resources: Arc<FakeResourcePort>,
    pub clock: Arc<FakeClockPort>,
    pub updates: broadcast::Sender<SessionUpdate>,
    pub selection: Arc<Mutex<NamespaceSelection>>,
    /// The cluster's current store; tests swap it for a reconnect or clear it for a disconnect.
    pub store: Arc<Mutex<Option<ResourceStore>>>,
    pub service: ReleasesService,
}

struct FixtureSource {
    helm: Arc<FakeHelmPort>,
    store: Arc<Mutex<Option<ResourceStore>>>,
    updates: broadcast::Sender<SessionUpdate>,
    selection: Arc<Mutex<NamespaceSelection>>,
}

impl ReleaseSource for FixtureSource {
    fn helm(&self, _: &ClusterId) -> Option<Arc<dyn HelmPort>> {
        Some(self.helm.clone())
    }

    fn namespaces(&self, _: &ClusterId) -> NamespaceSelection {
        self.selection.lock().clone()
    }

    fn store(&self, _: &ClusterId) -> Option<ResourceStore> {
        self.store.lock().clone()
    }

    fn kinds(&self, _: &ClusterId) -> BoxFuture<'static, OxiResult<Arc<KindScopes>>> {
        Box::pin(async { Ok(Arc::new(KindScopes::default())) })
    }

    fn session_updates(&self) -> BoxStream<'static, SessionUpdate> {
        futures::stream::unfold(self.updates.subscribe(), |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(update) => return Some((update, rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

impl Fixture {
    pub fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let store = Arc::new(Mutex::new(Some(store_on(&resources, &clock))));
        let helm = Arc::new(FakeHelmPort::native_only());
        let (updates, _) = broadcast::channel(16);
        let selection = Arc::new(Mutex::new(NamespaceSelection::All));
        let service = ReleasesService::new(Arc::new(FixtureSource {
            helm: helm.clone(),
            store: store.clone(),
            updates: updates.clone(),
            selection: selection.clone(),
        }));
        Self {
            helm,
            resources,
            clock,
            updates,
            selection,
            store,
            service,
        }
    }

    /// Lets the spawned feed tasks run.
    pub async fn settle(&self) {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }
}

/// A store over `resources`, its feeds spawned on the test's runtime.
pub(super) fn store_on(
    resources: &Arc<FakeResourcePort>,
    clock: &Arc<FakeClockPort>,
) -> ResourceStore {
    let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
    ResourceStore::new(
        cluster(),
        StorePorts {
            resources: resources.clone(),
            tables,
        },
        StoreRuntime {
            spawner: Arc::new(|task: BoxFuture<'static, ()>| {
                tokio::spawn(task);
            }),
            clock: clock.clone(),
            probe: None,
        },
        StoreOptions::default(),
    )
}

/// A release record Secret as the storage driver writes it (labels only; no payload needed
/// because the store watches metadata).
pub(super) fn record_secret(name: &str, revision: u32) -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": format!("sh.helm.release.v1.{name}.v{revision}"),
            "namespace": "apps",
            "uid": format!("uid-{name}-{revision}"),
            "labels": {"owner": "helm", "name": name, "version": revision.to_string(), "status": "deployed"},
        },
        "type": "helm.sh/release.v1",
    }))
    .expect("secret")
}
