//! `ResourceReader` and `TableFeedPort` for [`BudgetedResources`]: feeds through the budget,
//! everything else straight to the client.

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{ObjectMeta, OxiResult, Resource};
use oxikube_ports::{
    ListOptions, ListPage, ResourceReader, Scale, Subresource, Table, TableFeed, TableFeedPort,
    TableOptions, WatchFeed, WatchOptions,
};
use serde_json::Value;

use super::{BudgetedResources, table_request, watch_request, wrong_shape};
use crate::budget::source::FeedStream;
use crate::resources::namespace_of;

#[async_trait]
impl ResourceReader for BudgetedResources {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        self.resources.list(kind, namespace, options).await
    }

    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        self.resources.list_metadata(kind, namespace, options).await
    }

    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        self.resources.get(kind, namespace, name).await
    }

    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        self.resources.get_opt(kind, namespace, name).await
    }

    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        let request = watch_request(kind, namespace, options);
        let resources = &self.resources;
        let feed = self
            .feeds
            .open_owned(request, |bytes| async move {
                let scope = match namespace_of(namespace) {
                    Some(ns) => WatchScope::Namespaces(vec![ns.to_owned()]),
                    None => WatchScope::Cluster,
                };
                let feed = resources
                    .counting_bytes(bytes)
                    .reflector_feed(kind, &scope, options)
                    .await?;
                Ok(FeedStream::Resources(feed.into_watch_feed()))
            })
            .await?;
        feed.into_resources().ok_or_else(wrong_shape)
    }

    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale> {
        self.resources.get_scale(kind, namespace, name).await
    }

    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value> {
        self.resources
            .get_subresource(kind, namespace, name, subresource)
            .await
    }
}

#[async_trait]
impl TableFeedPort for BudgetedResources {
    async fn list_table(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<Table> {
        self.resources.list_table(kind, namespace, options).await
    }

    async fn table_feed(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<TableFeed> {
        let request = table_request(kind, namespace, options);
        let resources = &self.resources;
        let feed = self
            .feeds
            .open_owned(request, |bytes| async move {
                let feed = resources
                    .counting_bytes(bytes)
                    .open_table_feed(kind, namespace, options)
                    .await?;
                Ok(FeedStream::Table(feed))
            })
            .await?;
        feed.into_table().ok_or_else(wrong_shape)
    }
}
