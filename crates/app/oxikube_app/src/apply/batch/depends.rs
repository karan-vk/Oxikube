//! Which earlier document a later one needs. The server cannot rehearse (or accept) an object in
//! a namespace that does not exist yet, nor a custom resource of a kind no definition has made
//! yet; a buffer that creates the namespace or the definition first is the usual way to ship
//! them, so the later document waits for it instead of failing its dry run.
//!
//! The rule is the domain's ([`oxikube_domain::apply::depends_on`], the edges of an `ApplyPlan`),
//! applied here in buffer order.

use oxikube_domain::apply::depends_on;
use serde_json::{Value, json};

use crate::apply::manifest::Manifest;
use crate::apply::types::ApplyRequest;

/// For each document, every earlier document it needs (a custom resource in a new namespace needs
/// both the Namespace and the definition), in buffer order. A document without a namespace is
/// compared in its request's default one. A document that does not parse needs nothing (its dry
/// run reports why).
pub(super) fn dependencies(
    requests: &[ApplyRequest],
    manifests: &[Option<Manifest>],
) -> Vec<Vec<usize>> {
    let objects: Vec<Value> = manifests
        .iter()
        .zip(requests)
        .map(|(manifest, request)| match manifest {
            Some(manifest) => in_default_namespace(manifest, request.namespace.as_deref()),
            None => Value::Null,
        })
        .collect();
    let objects: Vec<&Value> = objects.iter().collect();
    depends_on(&objects)
}

/// The object, with `metadata.namespace` set to `default` when it names none.
fn in_default_namespace(manifest: &Manifest, default: Option<&str>) -> Value {
    let mut object = manifest.object().clone();
    if manifest.namespace.is_none()
        && let Some(namespace) = default
        && let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut)
    {
        metadata.insert("namespace".into(), json!(namespace));
    }
    object
}
