//! Single-object reads.

use kube::core::{ApiResource, DynamicObject};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult, Resource};

use super::backend::dynamic_json;
use super::error::{bad_object, get_error};
use super::{KubeResources, namespace_of};

impl KubeResources {
    /// A server object (a write's response) as a domain [`Resource`], with `managedFields`
    /// handled as for `get`.
    pub(crate) fn resource_of(
        &self,
        item: DynamicObject,
        resource: &ApiResource,
    ) -> OxiResult<Resource> {
        let strip = self.config.get_managed_fields.strips();
        Resource::from_json(dynamic_json(item, resource, strip))
            .map_err(|e| bad_object("object", e))
    }

    /// One object, `None` when the server answers 404.
    pub(super) async fn get_one(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::Get, true).await?;
        let api = self.kind_api(&resource, namespace);
        let strip = self.config.get_managed_fields.strips();
        let json = api.get_opt(name, strip).await.map_err(|e| get_error(&e))?;
        json.map(|json| Resource::from_json(json).map_err(|e| bad_object("object", e)))
            .transpose()
    }

    /// One object; a missing object is `NotFound` naming kind, namespace and name.
    pub(super) async fn get_required(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Resource> {
        self.get_one(kind, namespace, name).await?.ok_or_else(|| {
            let place = match namespace_of(namespace) {
                Some(ns) => format!("{ns}/{name}"),
                None => name.to_owned(),
            };
            OxiError::not_found(format!("{kind} {place} not found"))
        })
    }
}
