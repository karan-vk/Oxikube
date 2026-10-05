//! The `scale` subresource: read it, set `spec.replicas` through it.

use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{Scale, Subresource, WriteOptions};
use serde_json::Value;

use super::patches::ResourcePatch;
use crate::resources::KubeResources;

impl KubeResources {
    /// The `scale` of one object; see `ResourceReader::get_scale`.
    pub(crate) async fn read_scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Scale> {
        let value = self
            .read_subresource(kind, namespace, name, &Subresource::Scale)
            .await?;
        scale_of(&value)
    }

    /// Sets the replica count through the `scale` subresource; see `ResourceWriter::scale`.
    ///
    /// The subresource, not a patch of `spec.replicas` on the object, so the call works the
    /// same for Deployments, StatefulSets, ReplicaSets and any CRD that declares a scale
    /// subresource, and needs only the `scale` permission.
    pub(crate) async fn write_scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        replicas: i32,
        options: &WriteOptions,
    ) -> OxiResult<Scale> {
        if replicas < 0 {
            return Err(OxiError::validation("replicas cannot be negative"));
        }
        let value = self
            .patch_subresource_of(
                kind,
                namespace,
                name,
                &Subresource::Scale,
                &ResourcePatch::Scale(replicas).to_patch(),
                options,
            )
            .await?;
        scale_of(&value)
    }
}

/// A `Scale` object (`autoscaling/v1`) as the port type. Missing counts read as zero (a
/// freshly created object reports no `status.replicas`).
fn scale_of(value: &Value) -> OxiResult<Scale> {
    if !value.is_object() {
        return Err(OxiError::internal(
            "the cluster sent an invalid scale object: not a JSON object",
        ));
    }
    let count = |pointer: &str| -> OxiResult<i32> {
        match value.pointer(pointer).and_then(Value::as_i64) {
            None => Ok(0),
            Some(n) => i32::try_from(n).map_err(|_| {
                OxiError::internal("the cluster sent an invalid scale object: count out of range")
            }),
        }
    };
    let text = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Ok(Scale {
        replicas: count("/spec/replicas")?,
        current_replicas: count("/status/replicas")?,
        selector: text("/status/selector"),
        resource_version: text("/metadata/resourceVersion"),
    })
}
