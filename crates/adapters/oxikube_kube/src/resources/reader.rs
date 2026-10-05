//! `ResourceReader` for [`KubeResources`].

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiResult, Resource};
use oxikube_ports::{
    ListOptions, ListPage, ResourceReader, Scale, Subresource, WatchFeed, WatchOptions,
};
use serde_json::Value;

use oxikube_domain::session::WatchScope;

use super::{KubeResources, namespace_of};

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
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        let scope = match namespace_of(namespace) {
            Some(ns) => WatchScope::Namespaces(vec![ns.to_owned()]),
            None => WatchScope::Cluster,
        };
        Ok(self
            .reflector_feed(kind, &scope, options)
            .await?
            .into_watch_feed())
    }

    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale> {
        self.read_scale(kind, namespace, name).await
    }

    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value> {
        self.read_subresource(kind, namespace, name, subresource)
            .await
    }
}
