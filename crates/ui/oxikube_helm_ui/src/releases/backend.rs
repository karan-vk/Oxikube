//! [`ReleasesBackend`]: where the Releases page gets its data.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::LocalBoxStream;
use jiff::Timestamp;
use oxikube_app::helm::{ReleaseResource, ReleaseResourcesLease, ResourceState, UnknownReason};
use oxikube_app::helm_releases::{Disclosure, ManifestView, ReleasesService, ValuesView};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{Release, ReleaseRef, Revision};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::view::Health;
use tokio::sync::watch;

/// How the live cluster answers for one object of a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectState {
    /// It exists and its kind says it is healthy.
    Healthy,
    /// It exists and its kind says it is not (pending pod, unavailable workload).
    Unhealthy,
    /// It exists; its kind has no health rule.
    Present,
    /// The manifest lists it and the cluster does not have it.
    Missing,
    /// Nothing can be said yet (the kind's feed has not listed).
    Loading,
    /// The cluster does not serve the kind (a CRD that is not installed).
    NotServed,
}

/// One object of a release with its live state, as the Resources tab draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRow {
    /// Identity: cluster, kind, namespace, name.
    pub reference: ResourceRef,
    /// The template it was rendered from, when the manifest says.
    pub source: Option<String>,
    /// Its state in the cluster.
    pub state: ObjectState,
}

impl From<&ReleaseResource> for ObjectRow {
    fn from(resource: &ReleaseResource) -> Self {
        let state = match &resource.state {
            ResourceState::Present {
                health: Some(Health::Healthy),
                ..
            } => ObjectState::Healthy,
            ResourceState::Present {
                health: Some(Health::Unhealthy),
                ..
            } => ObjectState::Unhealthy,
            ResourceState::Present { health: None, .. } => ObjectState::Present,
            ResourceState::Missing => ObjectState::Missing,
            ResourceState::Unknown(UnknownReason::NotLoaded) => ObjectState::Loading,
            ResourceState::Unknown(UnknownReason::KindNotServed) => ObjectState::NotServed,
        };
        Self {
            reference: resource.reference.clone(),
            source: resource.source.clone(),
            state,
        }
    }
}

/// A claim on the feeds that serve one release's objects; dropping it releases them.
pub trait ObjectsLease: Send {
    /// The release's objects with their current state. Cheap: call it on each refresh.
    fn objects(&self) -> Vec<ObjectRow>;
}

impl ObjectsLease for ReleaseResourcesLease {
    fn objects(&self) -> Vec<ObjectRow> {
        self.resources().iter().map(ObjectRow::from).collect()
    }
}

/// The page's view of the app, bound to one cluster. Every `BoxFuture` is run on the runtime with
/// `spawn_kube`, never on the UI thread; nothing here waits.
pub trait ReleasesBackend: 'static {
    /// The cluster the page shows.
    fn cluster(&self) -> &ClusterId;

    /// The latest revision of every release in the tab's namespaces.
    fn list(&self) -> BoxFuture<'static, OxiResult<Vec<Release>>>;

    /// One item whenever the release records, the scope or the connection changed (list again).
    /// The live backend always has one, even before the cluster connects (it follows the store
    /// a connect builds); `None` means the backend has no live source (fixtures). Called on the
    /// UI thread.
    fn watch(&self) -> Option<LocalBoxStream<'static, ()>>;

    /// Every revision of `release`, newest first.
    fn history(&self, release: &ReleaseRef) -> BoxFuture<'static, OxiResult<Vec<Revision>>>;

    /// The values of `revision`; credential-looking ones masked unless the user revealed them.
    fn values(
        &self,
        release: &ReleaseRef,
        revision: u32,
        all: bool,
    ) -> BoxFuture<'static, OxiResult<ValuesView>>;

    /// The manifest of `revision`; Secret data masked unless the user revealed it.
    fn manifest(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<ManifestView>>;

    /// The notes of `revision`, `None` when the chart has none.
    fn notes(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<Option<Arc<str>>>>;

    /// The release's objects, kept current while the lease lives.
    fn objects(
        &self,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
    ) -> BoxFuture<'static, OxiResult<Box<dyn ObjectsLease>>>;

    /// Changes whenever a reveal was granted or withdrawn.
    fn reveal_changes(&self) -> watch::Receiver<u64>;

    /// Whether the user revealed `release`'s masked values.
    fn is_revealed(&self, release: &ReleaseRef) -> bool;

    /// Hides `release`'s values again (its detail closed).
    fn conceal(&self, release: &ReleaseRef);
}

/// The backend over the app's [`ReleasesService`].
#[derive(Clone)]
pub struct ServiceBackend {
    service: ReleasesService,
    cluster: ClusterId,
}

impl ServiceBackend {
    /// A backend showing `cluster`'s releases from `service`.
    pub fn new(service: ReleasesService, cluster: ClusterId) -> Self {
        Self { service, cluster }
    }
}

impl ReleasesBackend for ServiceBackend {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn list(&self) -> BoxFuture<'static, OxiResult<Vec<Release>>> {
        let (service, cluster) = (self.service.clone(), self.cluster.clone());
        Box::pin(async move { service.list(&cluster).await })
    }

    fn watch(&self) -> Option<LocalBoxStream<'static, ()>> {
        Some(self.service.watch(&self.cluster).boxed_local())
    }

    fn history(&self, release: &ReleaseRef) -> BoxFuture<'static, OxiResult<Vec<Revision>>> {
        let (service, release) = (self.service.clone(), release.clone());
        Box::pin(async move { service.history(&release).await })
    }

    fn values(
        &self,
        release: &ReleaseRef,
        revision: u32,
        all: bool,
    ) -> BoxFuture<'static, OxiResult<ValuesView>> {
        let (service, release) = (self.service.clone(), release.clone());
        Box::pin(async move {
            service
                .values(&release, Some(revision), all, Disclosure::Granted)
                .await
        })
    }

    fn manifest(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<ManifestView>> {
        let (service, release) = (self.service.clone(), release.clone());
        Box::pin(async move {
            service
                .manifest(&release, Some(revision), Disclosure::Granted)
                .await
        })
    }

    fn notes(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<Option<Arc<str>>>> {
        let (service, release) = (self.service.clone(), release.clone());
        Box::pin(async move { service.notes(&release, Some(revision)).await })
    }

    fn objects(
        &self,
        release: &ReleaseRef,
        revision: u32,
        updated: Timestamp,
    ) -> BoxFuture<'static, OxiResult<Box<dyn ObjectsLease>>> {
        let (service, release) = (self.service.clone(), release.clone());
        Box::pin(async move {
            let lease = service.resources(&release, revision, updated).await?;
            Ok(Box::new(lease) as Box<dyn ObjectsLease>)
        })
    }

    fn reveal_changes(&self) -> watch::Receiver<u64> {
        self.service.reveal_changes()
    }

    fn is_revealed(&self, release: &ReleaseRef) -> bool {
        self.service.is_revealed(release)
    }

    fn conceal(&self, release: &ReleaseRef) {
        self.service.conceal(release);
    }
}
