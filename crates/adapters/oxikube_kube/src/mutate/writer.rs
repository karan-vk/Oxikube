//! `ResourceWriter` for [`KubeResources`].
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

use crate::resources::KubeResources;

#[async_trait]
impl ResourceWriter for KubeResources {
    async fn create(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.create_object(kind, namespace, object, options).await
    }

    async fn replace(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.replace_object(kind, namespace, name, object, options)
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
        self.patch_object(kind, namespace, name, patch, options)
            .await
    }

    async fn delete(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        self.delete_object(kind, namespace, name, options).await
    }

    async fn delete_collection(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        self.delete_matching(kind, namespace, selection, options)
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
        self.write_scale(kind, namespace, name, replicas, options)
            .await
    }

    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()> {
        self.evict_pod(namespace, pod, options).await
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
        self.post_subresource(kind, namespace, name, subresource, body, options)
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
        self.patch_subresource_of(kind, namespace, name, subresource, patch, options)
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
        self.replace_subresource_of(kind, namespace, name, subresource, body, options)
            .await
    }
}
