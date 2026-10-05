//! The raw subresource requests: any subresource of any kind, bodies and responses as JSON.

use kube::Resource as _;
use kube::core::params::GetParams;
use kube::core::{DynamicObject, Request};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{Patch, Subresource, WriteOptions};
use serde_json::Value;
use tracing::debug;

use super::error::unsupported;
use crate::mutate::{patch_request, post_params, write_error};
use crate::resources::{KubeResources, namespace_of};

/// A path segment that cannot change the request's route: non-empty, no `/`, `?`, `#`, `%`
/// or whitespace.
pub(crate) fn segment(what: &str, value: &str) -> OxiResult<()> {
    let bad = |c: char| matches!(c, '/' | '?' | '#' | '%') || c.is_whitespace();
    if value.is_empty() || value.contains(bad) {
        return Err(OxiError::validation(format!(
            "{what} must be a plain path segment"
        )));
    }
    Ok(())
}

/// A checked subresource call: the request builder for the object's path and what an error
/// should name.
struct Target<'a> {
    request: Request,
    name: &'a str,
    what: String,
    subresource: &'a Subresource,
}

/// Whether `err` is an HTTP 404 from the API server.
fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.code == 404)
}

impl KubeResources {
    /// The target of a call on `kind`'s object `name`, after checking the kind is served,
    /// supports `verb`, fits `namespace`, and that `name` and `subresource` are plain path
    /// segments.
    async fn subresource_target<'a>(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &'a str,
        subresource: &'a Subresource,
        verb: Verb,
    ) -> OxiResult<Target<'a>> {
        if let Some(ns) = namespace {
            segment("a namespace", ns)?;
        }
        segment("an object name", name)?;
        segment("a subresource name", subresource.as_str())?;
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, verb, true).await?;
        Ok(Target {
            request: Request::new(DynamicObject::url_path(&resource, namespace)),
            name,
            what: kind.to_string(),
            subresource,
        })
    }

    /// Sends a built request and decodes the JSON response.
    ///
    /// A 404 is ambiguous (see `error`): the object is read to tell a missing object, which
    /// stays `NotFound`, from a subresource the kind does not serve, which is `Unsupported`.
    async fn send_subresource(
        &self,
        target: &Target<'_>,
        built: Result<http::Request<Vec<u8>>, kube::core::request::Error>,
    ) -> OxiResult<Value> {
        let built = built.map_err(|e| OxiError::validation(e.to_string()))?;
        match self.client().request::<Value>(built).await {
            Ok(value) => Ok(value),
            Err(err) if is_not_found(&err) => {
                let probe = target.request.get(target.name, &GetParams::default());
                let exists = match probe {
                    Ok(probe) => self.client().request::<Value>(probe).await.is_ok(),
                    Err(_) => false,
                };
                Err(if exists {
                    unsupported(&target.what, target.subresource)
                } else {
                    write_error(&err)
                })
            }
            Err(err) => Err(write_error(&err)),
        }
    }

    /// `GET .../{name}/{subresource}`.
    pub(crate) async fn read_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value> {
        let target = self
            .subresource_target(kind, namespace, name, subresource, Verb::Get)
            .await?;
        debug!(op = "get_subresource", %kind, namespace, name, %subresource, "subresource");
        let built = target.request.get_subresource(subresource.as_str(), name);
        self.send_subresource(&target, built).await
    }

    /// `POST .../{name}/{subresource}` with `body`.
    pub(crate) async fn post_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        let target = self
            .subresource_target(kind, namespace, name, subresource, Verb::Create)
            .await?;
        let params = post_params(options)?;
        debug!(
            op = "create_subresource", %kind, namespace, name, %subresource,
            dry_run = options.dry_run, "subresource"
        );
        let built =
            target
                .request
                .create_subresource(subresource.as_str(), name, &params, encode(body)?);
        self.send_subresource(&target, built).await
    }

    /// `PATCH .../{name}/{subresource}`.
    pub(crate) async fn patch_subresource_of(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        let target = self
            .subresource_target(kind, namespace, name, subresource, Verb::Patch)
            .await?;
        let (params, body) = patch_request(patch, options)?;
        debug!(
            op = "patch_subresource", %kind, namespace, name, %subresource,
            content_type = patch.kind.content_type(), dry_run = options.dry_run, "subresource"
        );
        let built = target
            .request
            .patch_subresource(subresource.as_str(), name, &params, &body);
        self.send_subresource(&target, built).await
    }

    /// `PUT .../{name}/{subresource}` with `body`.
    pub(crate) async fn replace_subresource_of(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        let target = self
            .subresource_target(kind, namespace, name, subresource, Verb::Update)
            .await?;
        let params = post_params(options)?;
        debug!(
            op = "replace_subresource", %kind, namespace, name, %subresource,
            dry_run = options.dry_run, "subresource"
        );
        let built =
            target
                .request
                .replace_subresource(subresource.as_str(), name, &params, encode(body)?);
        self.send_subresource(&target, built).await
    }
}

/// A request body as JSON bytes.
pub(super) fn encode(body: &Value) -> OxiResult<Vec<u8>> {
    serde_json::to_vec(body).map_err(|_| OxiError::validation("the request body is not valid JSON"))
}
