//! `ResourceReader` on kube-rs: paginated list and get for any kind (E04-S01).
//!
//! [`KubeResources`] serves one connected cluster. A [`Gvk`] is resolved through the shared
//! discovery registry to an `ApiResource` and a scope, so core kinds and CRDs take the same
//! code path. Two decoders sit behind one seam (`backend::KindApi`): `Api<DynamicObject>` for
//! everything (default, lossless) and `Api<K>` for the core kinds ([`AccessPath::Typed`]);
//! both yield the same domain [`Resource`](oxikube_domain::Resource). kube types never leave
//! this module (ADR 0005).
//!
//! | Piece | Where |
//! |---|---|
//! | `ResourceReader::{list, list_metadata, get, get_opt}` | `reader` (trait impl), `list`, `get` |
//! | page size, `managedFields`, typed/dynamic, 410 restarts | [`ResourcesConfig`] |
//! | `ListOptions` to `ListParams`, `resourceVersion` rules | `params` |
//! | [`KubeResources::list_all`]: pages until exhausted, restarts on a stale continue token | `list` |
//! | 410 Gone marker ([`is_list_expired`]) and error mapping | `error` |
//! | `TableFeedPort` (E04-S04), sharing `target`, `list_params` and `list_error` | [`crate::table`] |
//!
//! # Pagination and memory
//!
//! `list` is one request: a page of at most `ListOptions::limit` objects plus the continue
//! token and the list `resourceVersion` (what S02's feed starts a watch from). `list_all`
//! follows the tokens, converting each page as it arrives, so it holds one page of raw JSON
//! and the converted objects, never two copies of the whole list. Without a limit, `list_all`
//! uses [`ResourcesConfig::page_size`] (default 500); `resourceVersion` semantics are in
//! `params`.
//!
//! # Errors
//!
//! kube failures go through [`classify`](crate::auth::classify): 401 `Auth`, 403 `Forbidden`,
//! 404 `NotFound`, 429/503 `Network`, timeouts `Timeout`. A kind the cluster does not serve
//! (or serves without the verb) is `Unsupported`; a namespace on a cluster-scoped kind (or a
//! missing one for `get` on a namespaced kind) is `Validation`. HTTP 410 on a list is a
//! `Conflict` carrying the [`ListExpired`] marker.
//!
//! # Elsewhere, or not here yet
//!
//! `watch` is the reflector feed in [`crate::feed`] (E04-S02). `get_scale` and
//! `get_subresource` (E04-S06) answer `Unsupported` until their stories land; the writer half
//! of `ResourcePort` is E04-S05.

mod backend;
mod config;
mod error;
mod get;
mod list;
mod params;
mod reader;
#[cfg(test)]
mod tests;
mod typed;

use kube::Client;
use kube::core::ApiResource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult};

use crate::discovery::KubeDiscovery;
use crate::feed::FeedSettings;
use backend::{Dynamic, KindApi};

pub(crate) use backend::dynamic_json;
pub(crate) use error::list_error;

pub use config::{AccessPath, DEFAULT_PAGE_SIZE, ManagedFields, ResourcesConfig};
pub use error::{ListExpired, is_list_expired};
pub(crate) use error::{bad_object, list_error};
pub(crate) use params::{deadline, list_params};
pub(crate) use reader::pending;

/// Resource reads for one cluster. Cheap to clone; clones share the client and discovery.
#[derive(Clone)]
pub struct KubeResources {
    pub(crate) client: Client,
    pub(crate) discovery: KubeDiscovery,
    pub(crate) config: ResourcesConfig,
    /// Reflector feed settings (E04-S02, `crate::feed`).
    pub(crate) feeds: FeedSettings,
}

impl KubeResources {
    /// Reads through `client`, resolving kinds with `discovery` (built on the same client).
    pub fn new(client: Client, discovery: KubeDiscovery) -> Self {
        Self::with_config(client, discovery, ResourcesConfig::default())
    }

    /// As [`new`](Self::new) with explicit settings.
    pub fn with_config(client: Client, discovery: KubeDiscovery, config: ResourcesConfig) -> Self {
        Self {
            client,
            discovery,
            config,
            feeds: FeedSettings::default(),
        }
    }

    /// The client every request goes through (sibling modules such as `table` build their own).
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The settings in effect.
    pub fn config(&self) -> &ResourcesConfig {
        &self.config
    }

    /// Whether the cluster serves `gvk` with `verb`, per the current discovery snapshot.
    pub(crate) fn serves(&self, gvk: &Gvk, verb: Verb) -> bool {
        self.discovery
            .registry()
            .get(gvk)
            .is_some_and(|kind| kind.supports(verb))
    }

    /// Resolves `gvk` to its `ApiResource`, checks it supports `verb` and that `namespace` fits its scope.
    /// `namespace_required` is true for single-object calls on namespaced kinds.
    pub(crate) async fn target(
        &self,
        gvk: &Gvk,
        namespace: Option<&str>,
        verb: Verb,
        namespace_required: bool,
    ) -> OxiResult<ApiResource> {
        let resource = self
            .discovery
            .resolve_api_resource(gvk)
            .await?
            .ok_or_else(|| not_served(gvk))?;
        // Same registry snapshot lookup; a refresh between the two calls that drops the kind
        // is reported as not served.
        let kind = self
            .discovery
            .registry()
            .get(gvk)
            .cloned()
            .ok_or_else(|| not_served(gvk))?;
        if !kind.supports(verb) {
            return Err(OxiError::unsupported(format!(
                "{} does not support `{verb}` on this cluster",
                kind.gvk
            )));
        }
        match (kind.namespaced, namespace) {
            (false, Some(ns)) => Err(OxiError::validation(format!(
                "{} is cluster-scoped; got namespace `{ns}`",
                kind.gvk
            ))),
            (true, None) if namespace_required => Err(OxiError::validation(format!(
                "{} is namespaced; a namespace is required",
                kind.gvk
            ))),
            _ => Ok(resource),
        }
    }

    /// The API handle for a resolved kind: typed for the core kinds when configured,
    /// otherwise dynamic.
    fn kind_api(&self, resource: &ApiResource, namespace: Option<&str>) -> Box<dyn KindApi> {
        if self.config.access_path == AccessPath::Typed {
            if let Some(api) = typed::select(&self.client, resource, namespace) {
                return api;
            }
        }
        Box::new(self.dynamic(resource, namespace))
    }

    fn dynamic(&self, resource: &ApiResource, namespace: Option<&str>) -> Dynamic {
        Dynamic::new(self.client.clone(), resource, namespace)
    }
}

impl std::fmt::Debug for KubeResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeResources")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn not_served(gvk: &Gvk) -> OxiError {
    OxiError::unsupported(format!("the cluster does not serve {gvk}"))
}

/// Treats `Some("")` as no namespace.
pub(crate) fn namespace_of(namespace: Option<&str>) -> Option<&str> {
    namespace.filter(|ns| !ns.is_empty())
}
