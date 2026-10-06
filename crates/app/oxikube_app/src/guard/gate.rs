//! [`ReadOnlyGate`]: the second read-only check, right before each request.
//!
//! The guard checks the flag when a command is admitted. A long-running flow (a drain, an
//! apply of many objects, a handler waiting on its own confirmation) can outlive that check, so
//! the writer a handler receives is wrapped: every write re-reads the session's flag first and
//! refuses with `Forbidden` when read-only mode went on in the meantime. The read is one short
//! in-memory lock, so the gate costs nothing next to the request it guards.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, Patch, ResourceWriter,
    Scale, Subresource, WriteOptions,
};
use serde_json::Value;

use crate::session::ClusterSessionManager;

/// A [`ResourceWriter`] that refuses every call while its cluster is read-only.
pub(super) struct ReadOnlyGate {
    sessions: ClusterSessionManager,
    cluster: ClusterId,
    context: ContextName,
    inner: Arc<dyn ResourceWriter>,
}

impl ReadOnlyGate {
    pub(super) fn new(
        sessions: ClusterSessionManager,
        cluster: ClusterId,
        context: ContextName,
        inner: Arc<dyn ResourceWriter>,
    ) -> Self {
        Self {
            sessions,
            cluster,
            context,
            inner,
        }
    }

    fn check(&self) -> OxiResult<()> {
        if self.sessions.is_read_only(&self.cluster) {
            tracing::warn!(cluster = %self.context, "write refused: read-only mode went on while the command ran");
            return Err(OxiError::new(
                ErrorKind::Forbidden,
                format!("cluster {} is read-only", self.context),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ResourceWriter for ReadOnlyGate {
    async fn create(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner.create(kind, namespace, object, options).await
    }

    async fn replace(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner
            .replace(kind, namespace, name, object, options)
            .await
    }

    async fn patch(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner
            .patch(kind, namespace, name, patch, options)
            .await
    }

    async fn delete(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        self.check()?;
        self.inner.delete(kind, namespace, name, options).await
    }

    async fn delete_collection(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        self.check()?;
        self.inner
            .delete_collection(kind, namespace, selection, options)
            .await
    }

    async fn scale(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        replicas: i32,
        options: &WriteOptions,
    ) -> OxiResult<Scale> {
        self.check()?;
        self.inner
            .scale(kind, namespace, name, replicas, options)
            .await
    }

    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()> {
        self.check()?;
        self.inner.evict(namespace, pod, options).await
    }

    async fn create_subresource(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .create_subresource(kind, namespace, name, subresource, body, options)
            .await
    }

    async fn patch_subresource(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .patch_subresource(kind, namespace, name, subresource, patch, options)
            .await
    }

    async fn replace_subresource(
        &self,
        kind: &oxikube_domain::ids::Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .replace_subresource(kind, namespace, name, subresource, body, options)
            .await
    }
}
