//! `ResourceReader` for [`KubeResources`].

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::{
    ListOptions, ListPage, ResourceReader, Scale, Subresource, WatchFeed, WatchOptions,
};
use serde_json::Value;

use super::KubeResources;

#[async_trait]
impl ResourceReader for KubeResources {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        self.list_page(kind, namespace, options).await
    }

    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        self.list_meta_page(kind, namespace, options).await
    }

    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        self.get_required(kind, namespace, name).await
    }

    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        self.get_one(kind, namespace, name).await
    }

    async fn watch(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        Err(pending("watch", "E04-S02"))
    }

    async fn get_scale(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
    ) -> OxiResult<Scale> {
        Err(pending("get_scale", "E04-S06"))
    }

    async fn get_subresource(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
        _subresource: &Subresource,
    ) -> OxiResult<Value> {
        Err(pending("get_subresource", "E04-S06"))
    }
}

/// A port method whose story has not landed yet.
fn pending(method: &str, story: &str) -> OxiError {
    OxiError::unsupported(format!("`{method}` is not implemented yet ({story})"))
}
