//! [`ReadOnlyGate`]: the second read-only check, right before each request.
//!
//! The guard checks the flag when a command is admitted. A long-running flow (a drain, an
//! apply of many objects, a handler waiting on its own confirmation) can outlive that check, so
//! the writer a handler receives is wrapped: every write re-reads the session's flag first and
//! refuses with `Forbidden` when read-only mode went on in the meantime. The read is one short
//! in-memory lock, so the gate costs nothing next to the request it guards.
//!
//! The writer the guard hands a handler for its **rehearsal** (the mandatory dry run before an
//! `Irreversible` command is confirmed, see `rehearsal`) is the same gate in rehearsal mode: it
//! also forces `dry_run` on every request, so a rehearsal cannot change the cluster even if a
//! handler forgot to pass [`Mutation::write_options`](super::Mutation::write_options).

use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
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
    /// Every request is sent as a server-side dry run, whatever its options say.
    rehearsal: bool,
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
            rehearsal: false,
        }
    }

    /// The gate in rehearsal mode: it also sends every request as a server-side dry run.
    pub(super) fn rehearsal(
        sessions: ClusterSessionManager,
        cluster: ClusterId,
        context: ContextName,
        inner: Arc<dyn ResourceWriter>,
    ) -> Self {
        Self {
            rehearsal: true,
            ..Self::new(sessions, cluster, context, inner)
        }
    }

    /// `options`, as a dry run in rehearsal mode.
    fn write_options<'a>(&self, options: &'a WriteOptions) -> Cow<'a, WriteOptions> {
        if self.rehearsal && !options.dry_run {
            Cow::Owned(WriteOptions {
                dry_run: true,
                ..options.clone()
            })
        } else {
            Cow::Borrowed(options)
        }
    }

    /// `options`, as a dry run in rehearsal mode.
    fn delete_options<'a>(&self, options: &'a DeleteOptions) -> Cow<'a, DeleteOptions> {
        if self.rehearsal && !options.dry_run {
            Cow::Owned(DeleteOptions {
                dry_run: true,
                ..options.clone()
            })
        } else {
            Cow::Borrowed(options)
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
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner
            .create(kind, namespace, object, &self.write_options(options))
            .await
    }

    async fn replace(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner
            .replace(kind, namespace, name, object, &self.write_options(options))
            .await
    }

    async fn patch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.check()?;
        self.inner
            .patch(kind, namespace, name, patch, &self.write_options(options))
            .await
    }

    async fn delete(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        self.check()?;
        self.inner
            .delete(kind, namespace, name, &self.delete_options(options))
            .await
    }

    async fn delete_collection(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        self.check()?;
        self.inner
            .delete_collection(kind, namespace, selection, &self.delete_options(options))
            .await
    }

    async fn scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        replicas: i32,
        options: &WriteOptions,
    ) -> OxiResult<Scale> {
        self.check()?;
        self.inner
            .scale(
                kind,
                namespace,
                name,
                replicas,
                &self.write_options(options),
            )
            .await
    }

    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()> {
        self.check()?;
        self.inner
            .evict(namespace, pod, &self.delete_options(options))
            .await
    }

    async fn create_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .create_subresource(
                kind,
                namespace,
                name,
                subresource,
                body,
                &self.write_options(options),
            )
            .await
    }

    async fn patch_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .patch_subresource(
                kind,
                namespace,
                name,
                subresource,
                patch,
                &self.write_options(options),
            )
            .await
    }

    async fn replace_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.check()?;
        self.inner
            .replace_subresource(
                kind,
                namespace,
                name,
                subresource,
                body,
                &self.write_options(options),
            )
            .await
    }
}
