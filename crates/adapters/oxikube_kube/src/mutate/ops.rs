//! The write requests: one `Api<DynamicObject>` call each, with the checks that need no server.

use either::Either;
use kube::core::{ApiResource, DynamicObject};
use kube::{Api, Client};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{
    DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, ListOptions, Patch, WriteOptions,
};
use serde_json::{Map, Value};
use tracing::debug;

use super::error::write_error;
use super::params::{delete_params, patch_request, post_params, selection_params};
use crate::resources::{KubeResources, namespace_of};

/// The API handle for one kind in one scope.
fn api(client: &Client, resource: &ApiResource, namespace: Option<&str>) -> Api<DynamicObject> {
    match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, resource),
        None => Api::all_with(client.clone(), resource),
    }
}

/// `object` as the request body: a JSON object whose `apiVersion` and `kind` default to the
/// resolved kind's. The error says what is wrong with the shape, never what the values are
/// (the object may be a Secret).
fn manifest(object: &Value, resource: &ApiResource) -> OxiResult<DynamicObject> {
    let Value::Object(fields) = object else {
        return Err(OxiError::validation("the manifest must be a JSON object"));
    };
    let mut fields: Map<String, Value> = fields.clone();
    for (key, default) in [
        ("apiVersion", &resource.api_version),
        ("kind", &resource.kind),
    ] {
        let missing = fields
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty);
        if missing {
            fields.insert(key.to_owned(), Value::String(default.clone()));
        }
    }
    serde_json::from_value(Value::Object(fields)).map_err(|_| {
        OxiError::validation("the manifest's apiVersion, kind or metadata has the wrong shape")
    })
}

impl KubeResources {
    /// Creates an object; see [`ResourceWriter::create`](oxikube_ports::ResourceWriter::create).
    pub(super) async fn create_object(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::Create, true).await?;
        let params = post_params(options)?;
        let body = manifest(object, &resource)?;
        debug!(op = "create", %kind, namespace, dry_run = options.dry_run, "mutation");
        let created = api(self.client(), &resource, namespace)
            .create(&params, &body)
            .await
            .map_err(|e| write_error(&e))?;
        self.resource_of(created, &resource)
    }

    /// Replaces an object; see
    /// [`ResourceWriter::replace`](oxikube_ports::ResourceWriter::replace).
    pub(super) async fn replace_object(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::Update, true).await?;
        let params = post_params(options)?;
        let body = manifest(object, &resource)?;
        debug!(op = "replace", %kind, namespace, name, dry_run = options.dry_run, "mutation");
        let replaced = api(self.client(), &resource, namespace)
            .replace(name, &params, &body)
            .await
            .map_err(|e| write_error(&e))?;
        self.resource_of(replaced, &resource)
    }

    /// Patches an object (merge, strategic, JSON patch or server-side apply); see
    /// [`ResourceWriter::patch`](oxikube_ports::ResourceWriter::patch).
    pub(super) async fn patch_object(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::Patch, true).await?;
        let (params, body) = patch_request(patch, options)?;
        debug!(
            op = "patch", %kind, namespace, name, content_type = patch.kind.content_type(),
            dry_run = options.dry_run, force = params.force, manager = params.field_manager.as_deref(),
            "mutation"
        );
        let patched = api(self.client(), &resource, namespace)
            .patch(name, &params, &body)
            .await
            .map_err(|e| write_error(&e))?;
        self.resource_of(patched, &resource)
    }

    /// Deletes an object; see [`ResourceWriter::delete`](oxikube_ports::ResourceWriter::delete).
    pub(super) async fn delete_object(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::Delete, true).await?;
        debug!(op = "delete", %kind, namespace, name, dry_run = options.dry_run, "mutation");
        let outcome = api(self.client(), &resource, namespace)
            .delete(name, &delete_params(options))
            .await
            .map_err(|e| write_error(&e))?;
        match outcome {
            Either::Left(remaining) => Ok(DeleteOutcome::Deleting(
                self.resource_of(remaining, &resource)?,
            )),
            Either::Right(_) => Ok(DeleteOutcome::Deleted),
        }
    }

    /// Deletes every object matching `selection`; see
    /// [`ResourceWriter::delete_collection`](oxikube_ports::ResourceWriter::delete_collection).
    ///
    /// A namespaced kind needs a namespace: the API server has no cluster-wide
    /// `deletecollection` for them.
    pub(super) async fn delete_matching(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        let namespace = namespace_of(namespace);
        let resource = self
            .target(kind, namespace, Verb::DeleteCollection, true)
            .await?;
        debug!(
            op = "delete_collection", %kind, namespace,
            labels = selection.label_selector.as_deref(), fields = selection.field_selector.as_deref(),
            dry_run = options.dry_run, "mutation"
        );
        let outcome = api(self.client(), &resource, namespace)
            .delete_collection(&delete_params(options), &selection_params(selection))
            .await
            .map_err(|e| write_error(&e))?;
        match outcome {
            Either::Left(list) => {
                let items = list
                    .items
                    .into_iter()
                    .map(|item| self.resource_of(item, &resource))
                    .collect::<OxiResult<Vec<_>>>()?;
                Ok(DeleteCollectionOutcome::Deleting(items))
            }
            Either::Right(_) => Ok(DeleteCollectionOutcome::Deleted),
        }
    }
}
