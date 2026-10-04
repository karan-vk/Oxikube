//! The two ways to talk to the API server, behind one trait.
//!
//! [`KindApi`] is the single seam: `list` and `get_opt` hand back plain JSON
//! [`Value`]s shaped like the server's objects (`apiVersion`, `kind`, `metadata`, ...). The
//! dynamic implementation wraps `Api<DynamicObject>`; [`typed`](super::typed) wraps `Api<K>`
//! for core kinds. Callers cannot tell them apart, which the kind golden test pins.

use async_trait::async_trait;
use kube::api::{ListParams, ObjectList};
use kube::core::{ApiResource, DynamicObject, PartialObjectMeta};
use kube::{Api, Client};
use serde_json::{Map, Value};

/// One page as the server returned it, items already converted to JSON. No `Debug`: the items
/// may be Secrets.
pub(super) struct RawPage {
    pub(super) items: Vec<Value>,
    pub(super) continue_token: Option<String>,
    pub(super) resource_version: Option<String>,
    pub(super) remaining_item_count: Option<i64>,
}

impl RawPage {
    pub(super) fn from_list<T: Clone>(
        list: ObjectList<T>,
        convert: impl FnMut(T) -> Value,
    ) -> Self {
        let ObjectList {
            metadata, items, ..
        } = list;
        Self {
            items: items.into_iter().map(convert).collect(),
            continue_token: metadata.continue_.filter(|t| !t.is_empty()),
            resource_version: metadata.resource_version,
            remaining_item_count: metadata.remaining_item_count,
        }
    }
}

/// Reads one kind in one scope. Implemented for dynamic and typed access.
#[async_trait]
pub(super) trait KindApi: Send + Sync {
    /// One page of objects. `strip_managed_fields` drops `metadata.managedFields` before the
    /// object is serialised.
    async fn list(&self, params: &ListParams, strip_managed_fields: bool) -> kube::Result<RawPage>;

    /// One object, `None` when the server answers 404.
    async fn get_opt(&self, name: &str, strip_managed_fields: bool) -> kube::Result<Option<Value>>;
}

/// `Api<DynamicObject>` for any kind, built from a discovered [`ApiResource`].
pub(super) struct Dynamic {
    api: Api<DynamicObject>,
    resource: ApiResource,
}

impl Dynamic {
    /// `namespace` is `None` for cluster-scoped kinds and all-namespaces lists.
    pub(super) fn new(client: Client, resource: &ApiResource, namespace: Option<&str>) -> Self {
        let api = match namespace {
            Some(ns) => Api::namespaced_with(client, ns, resource),
            None => Api::all_with(client, resource),
        };
        Self {
            api,
            resource: resource.clone(),
        }
    }

    /// One page of `PartialObjectMetadata` as `{apiVersion, kind, metadata}` objects.
    pub(super) async fn list_metadata(&self, params: &ListParams) -> kube::Result<RawPage> {
        let list: ObjectList<PartialObjectMeta<DynamicObject>> =
            self.api.list_metadata(params).await?;
        let resource = &self.resource;
        Ok(RawPage::from_list(list, |item| {
            let mut object = type_fields(item.types.as_ref(), resource);
            object.insert("metadata".into(), meta_json(&item.metadata));
            Value::Object(object)
        }))
    }
}

#[async_trait]
impl KindApi for Dynamic {
    async fn list(&self, params: &ListParams, strip: bool) -> kube::Result<RawPage> {
        let list = self.api.list(params).await?;
        Ok(RawPage::from_list(list, |item| {
            dynamic_json(item, &self.resource, strip)
        }))
    }

    async fn get_opt(&self, name: &str, strip: bool) -> kube::Result<Option<Value>> {
        Ok(self
            .api
            .get_opt(name)
            .await?
            .map(|item| dynamic_json(item, &self.resource, strip)))
    }
}

/// `apiVersion` and `kind` as the server sent them, or from discovery when it did not: list
/// items carry no type fields.
fn type_fields(types: Option<&kube::core::TypeMeta>, resource: &ApiResource) -> Map<String, Value> {
    let (api_version, kind) = match types {
        Some(t) if !t.api_version.is_empty() && !t.kind.is_empty() => {
            (t.api_version.clone(), t.kind.clone())
        }
        _ => (resource.api_version.clone(), resource.kind.clone()),
    };
    let mut object = Map::new();
    object.insert("apiVersion".into(), Value::String(api_version));
    object.insert("kind".into(), Value::String(kind));
    object
}

fn meta_json(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> Value {
    // `ObjectMeta` only holds serialisable plain data, so this cannot fail; an empty object
    // would be rejected downstream as a missing name rather than hidden.
    serde_json::to_value(meta).unwrap_or_else(|_| Value::Object(Map::new()))
}

/// `DynamicObject` to the server's JSON shape, consuming it (no copy of `spec`/`status`).
/// Key order is `apiVersion`, `kind`, `metadata`, then the rest.
fn dynamic_json(mut item: DynamicObject, resource: &ApiResource, strip: bool) -> Value {
    if strip {
        item.metadata.managed_fields = None;
    }
    let mut object = type_fields(item.types.as_ref(), resource);
    object.insert("metadata".into(), meta_json(&item.metadata));
    if let Value::Object(rest) = item.data {
        for (key, value) in rest {
            object.entry(key).or_insert(value);
        }
    }
    Value::Object(object)
}
