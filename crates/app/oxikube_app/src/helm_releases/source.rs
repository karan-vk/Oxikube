//! [`ReleaseSource`]: what the releases service needs from the binary.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::HelmPort;

use crate::helm::KindScopes;
use crate::session::SessionUpdate;
use crate::store::ResourceStore;

/// The per-cluster inputs of [`ReleasesService`](super::ReleasesService). The binary implements
/// it over the sessions, the per-cluster stores and the native reader; tests over fakes.
///
/// Every method is cheap and non-blocking: it hands out handles, it reads nothing.
pub trait ReleaseSource: Send + Sync {
    /// The native Helm reader of `cluster`'s connection, `None` while it is not connected. The
    /// same `Arc` is returned for as long as the connection lives (the reader's cache is keyed
    /// by object, so a new reader would decode again).
    fn helm(&self, cluster: &ClusterId) -> Option<Arc<dyn HelmPort>>;

    /// The namespaces the cluster's tab watches.
    fn namespaces(&self, cluster: &ClusterId) -> NamespaceSelection;

    /// The cluster's resource store, `None` while it is not connected.
    fn store(&self, cluster: &ClusterId) -> Option<ResourceStore>;

    /// What discovery says about the kinds a manifest may use.
    fn kinds(&self, cluster: &ClusterId) -> BoxFuture<'static, OxiResult<Arc<KindScopes>>>;

    /// Every session update (namespace selection, connection state), so the watch can follow.
    fn session_updates(&self) -> BoxStream<'static, SessionUpdate>;
}
