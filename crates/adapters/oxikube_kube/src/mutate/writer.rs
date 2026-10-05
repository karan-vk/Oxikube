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

use crate::resources::{KubeResources, pending};

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
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
        _replicas: i32,
        _options: &WriteOptions,
    ) -> OxiResult<Scale> {
        Err(pending("scale", "E04-S06"))
    }

    async fn evict(&self, _namespace: &str, _pod: &str, _options: &DeleteOptions) -> OxiResult<()> {
        Err(pending("evict", "E04-S06"))
    }

    async fn create_subresource(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
        _subresource: &Subresource,
        _body: &Value,
        _options: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(pending("create_subresource", "E04-S06"))
    }

    async fn patch_subresource(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
        _subresource: &Subresource,
        _patch: &Patch,
        _options: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(pending("patch_subresource", "E04-S06"))
    }

    async fn replace_subresource(
        &self,
        _kind: &Gvk,
        _namespace: Option<&str>,
        _name: &str,
        _subresource: &Subresource,
        _body: &Value,
        _options: &WriteOptions,
    ) -> OxiResult<Value> {
        Err(pending("replace_subresource", "E04-S06"))
    }
}
