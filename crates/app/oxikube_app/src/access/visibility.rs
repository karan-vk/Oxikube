//! [`AccessVisibility`]: the RBAC implementation of the sidebar's [`KindVisibility`] hook.

use futures::future::BoxFuture;
use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::ids::{ClusterId, Gvk, Gvr};
use oxikube_domain::session::NamespaceSelection;

use super::{AccessRegistry, AccessService};
use crate::ClusterSessionManager;
use crate::kinds::KindVisibility;

/// Hides the kinds the user may not `list`, from the cached rules reviews of the cluster's
/// [`AccessService`].
///
/// Only a definite deny answers `Some(false)`: a review that has not arrived, a partial one, a
/// grant on named objects only and a namespaced kind under "all namespaces" (the cluster-wide
/// review sees one probe namespace) answer `None`, which shows the kind. The server stays the
/// authority; this only spares the user a sidebar of 403s.
///
/// [`is_listable_resource`](KindVisibility::is_listable_resource) reads the cache (stale
/// reviews included, so an entry does not reappear when its review ages) and never waits.
/// [`refresh`](KindVisibility::refresh) fills it, off the UI thread, after connect and on every
/// namespace change.
#[derive(Clone, Debug)]
pub struct AccessVisibility {
    sessions: ClusterSessionManager,
    access: AccessRegistry,
}

impl AccessVisibility {
    /// The hook over the services `access` hands out for the sessions of `sessions`.
    pub fn new(sessions: ClusterSessionManager, access: AccessRegistry) -> Self {
        Self { sessions, access }
    }

    fn service(&self, cluster: &ClusterId) -> Option<AccessService> {
        let session = self.sessions.get(cluster)?;
        self.access.for_session(cluster, &session)
    }
}

impl KindVisibility for AccessVisibility {
    fn is_listable_resource(
        &self,
        cluster: &ClusterId,
        gvk: &Gvk,
        plural: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<bool> {
        let service = self.service(cluster)?;
        let mut check = AccessCheck::new("list", Gvr::new(&*gvk.group, &*gvk.version, plural?));
        if let Some(namespace) = namespace {
            check = check.namespace(namespace);
        }
        match service.peek_stale(&check)? {
            AccessDecision::Allowed => Some(true),
            AccessDecision::Denied { .. } => Some(false),
            AccessDecision::Unknown { .. } => None,
        }
    }

    fn refresh<'a>(
        &'a self,
        cluster: &'a ClusterId,
        namespaces: &'a NamespaceSelection,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(service) = self.service(cluster) else {
                return;
            };
            // The cluster-wide review covers the cluster-scoped kinds; each selected namespace
            // adds its Roles. A failure leaves the cache as it was: the sidebar fails open.
            if let Err(error) = service.rules(None).await {
                tracing::debug!(%cluster, %error, "cluster-wide rules review failed");
            }
            for namespace in namespaces.names() {
                if let Err(error) = service.rules(Some(namespace)).await {
                    tracing::debug!(%cluster, namespace, %error, "namespace rules review failed");
                }
            }
        })
    }
}
