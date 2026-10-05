//! Eviction: a `policy/v1` `Eviction` posted to `pods/{name}/eviction`.
//!
//! Unlike a delete, an eviction honours PodDisruptionBudgets. kube's `Api::evict` is not used:
//! it serialises the delete options as `delete_options` (snake case), which the server ignores,
//! so grace period, preconditions and dry run would be silently lost. The `Eviction` is built
//! from the `k8s-openapi` type instead, so the body is `apiVersion: policy/v1, kind: Eviction`
//! with `deleteOptions` in the field the server reads.

use k8s_openapi::api::policy::v1::Eviction;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{
    DeleteOptions as MetaDeleteOptions, ObjectMeta, Preconditions,
};
use kube::api::PostParams;
use kube::core::{DynamicObject, Request};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DeleteOptions, PropagationPolicy};
use tracing::debug;

use super::error::evict_error;
use super::request::segment;
use crate::resources::KubeResources;
use kube::Resource as _;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The `Eviction` for `pod` with `options` as its delete options.
fn eviction(namespace: &str, pod: &str, options: &DeleteOptions) -> Eviction {
    let propagation = options.propagation.map(|p| {
        match p {
            PropagationPolicy::Orphan => "Orphan",
            PropagationPolicy::Background => "Background",
            PropagationPolicy::Foreground => "Foreground",
        }
        .to_owned()
    });
    let preconditions = options.preconditions.as_ref().map(|p| Preconditions {
        resource_version: p.resource_version.clone(),
        uid: p.uid.clone(),
    });
    Eviction {
        metadata: ObjectMeta {
            name: Some(pod.to_owned()),
            namespace: Some(namespace.to_owned()),
            ..ObjectMeta::default()
        },
        delete_options: Some(MetaDeleteOptions {
            dry_run: options.dry_run.then(|| vec!["All".to_owned()]),
            grace_period_seconds: options.grace_period_secs.map(i64::from),
            preconditions,
            propagation_policy: propagation,
            ..MetaDeleteOptions::default()
        }),
    }
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
        let body = serde_json::to_vec(&eviction(namespace, pod, options))
            .map_err(|_| OxiError::internal("the eviction could not be serialised"))?;
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
            .request::<serde_json::Value>(request)
            .await
            .map(drop)
            .map_err(|e| evict_error(&e, namespace, pod))
    }
}
