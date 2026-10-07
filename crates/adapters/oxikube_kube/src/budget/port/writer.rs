//! `ResourceWriter` for [`BudgetedResources`]: every write goes straight to the client. The
//! budget only gates feeds.
//!
//! **Mutating: reachable only through `MutationGuard`** (non-negotiable 3).

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, Patch, ResourceWriter,
    Scale, Subresource, WriteOptions,
};
use serde_json::Value;

use super::BudgetedResources;

#[async_trait]
impl ResourceWriter for BudgetedResources {
    async fn create(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        ResourceWriter::create(&self.resources, kind, namespace, object, options).await
    }

    async fn replace(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        ResourceWriter::replace(&self.resources, kind, namespace, name, object, options).await
    }

    async fn patch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        ResourceWriter::patch(&self.resources, kind, namespace, name, patch, options).await
    }

    async fn delete(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        ResourceWriter::delete(&self.resources, kind, namespace, name, options).await
    }

    async fn delete_collection(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        ResourceWriter::delete_collection(&self.resources, kind, namespace, selection, options)
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
        ResourceWriter::scale(&self.resources, kind, namespace, name, replicas, options).await
    }

    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()> {
        ResourceWriter::evict(&self.resources, namespace, pod, options).await
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
        ResourceWriter::create_subresource(
            &self.resources,
            kind,
            namespace,
            name,
            subresource,
            body,
            options,
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
        ResourceWriter::patch_subresource(
            &self.resources,
            kind,
            namespace,
            name,
            subresource,
            patch,
            options,
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
        ResourceWriter::replace_subresource(
            &self.resources,
            kind,
            namespace,
            name,
            subresource,
            body,
            options,
        )
        .await
    }
}
