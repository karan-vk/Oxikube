//! Eviction: a `policy/v1` `Eviction` posted to `pods/{name}/eviction`.
//!
//! Unlike a delete, an eviction honours PodDisruptionBudgets. kube's `Api::evict` is not used:
//! it serialises the delete options as `delete_options` (snake case), which the server ignores,
//! so grace period, preconditions and dry run would be silently lost. The body is built here
//! instead, with `deleteOptions` in the field the server reads.

use kube::Resource as _;
use kube::api::PostParams;
use kube::core::{DynamicObject, Request};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::DeleteOptions;
use serde_json::{Value, json};
use tracing::debug;

use super::error::evict_error;
use super::request::{encode, segment};
use crate::mutate::delete_params;
use crate::resources::KubeResources;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The `policy/v1` `Eviction` for `pod`, with `options` as its `deleteOptions` (built from the
/// same `DeleteParams` as a plain delete, so both agree on the field names).
fn eviction(namespace: &str, pod: &str, options: &DeleteOptions) -> Value {
    json!({
        "apiVersion": "policy/v1",
        "kind": "Eviction",
        "metadata": {"name": pod, "namespace": namespace},
        "deleteOptions": delete_params(options),
    })
}

impl KubeResources {
    /// Evicts `namespace/pod`; see `ResourceWriter::evict`.
    ///
    /// A refusal by a PodDisruptionBudget is the retryable error described in
    /// [`subresource`](crate::subresource#errors) carrying an
    /// [`EvictionBlocked`](super::EvictionBlocked) marker.
    pub(crate) async fn evict_pod(
        &self,
        namespace: &str,
        pod: &str,
        options: &DeleteOptions,
    ) -> OxiResult<()> {
        segment("a namespace", namespace)?;
        segment("a pod name", pod)?;
        let resource = self
            .target(&pod_gvk(), Some(namespace), Verb::Delete, true)
            .await?;
        // An eviction is an action, not a field write: no field manager.
        let params = PostParams {
            dry_run: options.dry_run,
            field_manager: None,
        };
        let body = encode(&eviction(namespace, pod, options))?;
        debug!(
            op = "evict",
            namespace,
            pod,
            dry_run = options.dry_run,
            "subresource"
        );
        let request = Request::new(DynamicObject::url_path(&resource, Some(namespace)))
            .create_subresource("eviction", pod, &params, body)
            .map_err(|e| OxiError::validation(e.to_string()))?;
        self.unretried_client()
            .request::<Value>(request)
            .await
            .map(drop)
            .map_err(|e| evict_error(&e, namespace, pod))
    }
}
