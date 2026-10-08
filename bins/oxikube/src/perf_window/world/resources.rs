//! [`WorldResources`]: the `ResourcePort` of a synthetic cluster. Pods and events come from its
//! [`Hub`] (live, churning); every other kind from a testkit store of fixed objects (namespaces,
//! nodes, workloads, the large ConfigMap). Read-only: every write is refused.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, ListPage, Patch,
    ResourceReader, ResourceWriter, Scale, Subresource, WatchFeed, WatchOptions, WriteOptions,
};
use oxikube_testkit::FakeResourcePort;
use serde_json::Value;

use super::hub::{Hub, Stream};

/// See the [module docs](self).
pub struct WorldResources {
    hub: Arc<Hub>,
    fixed: Arc<FakeResourcePort>,
}

impl WorldResources {
    /// The port over `hub` (pods, events) and `fixed` (everything else).
    pub fn new(hub: Arc<Hub>, fixed: Arc<FakeResourcePort>) -> Self {
        Self { hub, fixed }
    }

    fn live(kind: &Gvk) -> Option<Stream> {
        match (
            kind.group.as_ref(),
            kind.version.as_ref(),
            kind.kind.as_ref(),
        ) {
            ("", "v1", "Pod") => Some(Stream::Pods),
            ("", "v1", "Event") => Some(Stream::Events),
            _ => None,
        }
    }

    fn find(&self, stream: Stream, namespace: Option<&str>, name: &str) -> Option<Resource> {
        self.hub
            .list(stream, namespace)
            .into_iter()
            .find(|o| o.name() == name)
    }
}

fn read_only() -> OxiError {
    OxiError::forbidden("the synthetic perf cluster is read-only")
}

#[async_trait]
impl ResourceReader for WorldResources {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        match Self::live(kind) {
            Some(stream) => Ok(ListPage::complete(self.hub.list(stream, namespace))),
            None => self.fixed.list(kind, namespace, options).await,
        }
    }

    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        match Self::live(kind) {
            Some(stream) => Ok(ListPage::complete(
                self.hub
                    .list(stream, namespace)
                    .into_iter()
                    .map(|o| o.meta)
                    .collect(),
            )),
            None => self.fixed.list_metadata(kind, namespace, options).await,
        }
    }

    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        match Self::live(kind) {
            Some(stream) => self
                .find(stream, namespace, name)
                .ok_or_else(|| OxiError::not_found(format!("{} {name} not found", kind.kind))),
            None => self.fixed.get(kind, namespace, name).await,
        }
    }

    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        match Self::live(kind) {
            Some(stream) => Ok(self.find(stream, namespace, name)),
            None => self.fixed.get_opt(kind, namespace, name).await,
        }
    }

    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        match Self::live(kind) {
            Some(stream) => Ok(self.hub.watch(stream, namespace, options.metadata_only)),
            None => self.fixed.watch(kind, namespace, options).await,
        }
    }

    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale> {
        self.fixed.get_scale(kind, namespace, name).await
    }

    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value> {
        self.fixed
            .get_subresource(kind, namespace, name, subresource)
            .await
    }
}

#[async_trait]
impl ResourceWriter for WorldResources {
    async fn create(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        Err(read_only())
    }

    async fn replace(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        Err(read_only())
    }

    async fn patch(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Patch,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        Err(read_only())
    }

    async fn delete(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        Err(read_only())
    }

    async fn delete_collection(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &ListOptions,
        _: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        Err(read_only())
    }

    async fn scale(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: i32,
        _: &WriteOptions,
    ) -> OxiResult<Scale> {
        Err(read_only())
    }

    async fn evict(&self, _: &str, _: &str, _: &DeleteOptions) -> OxiResult<()> {
        Err(read_only())
    }

    async fn create_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(read_only())
    }

    async fn patch_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Patch,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(read_only())
    }

    async fn replace_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(read_only())
    }
}
