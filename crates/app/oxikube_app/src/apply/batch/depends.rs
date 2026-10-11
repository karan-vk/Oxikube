//! Which earlier document a later one needs. The server cannot rehearse (or accept) an object in
//! a namespace that does not exist yet, nor a custom resource of a kind no definition has made
//! yet; a buffer that creates the namespace or the definition first is the usual way to ship
//! them, so the later document waits for it instead of failing its dry run.

use oxikube_domain::ids::Gvk;
use serde_json::Value;

use crate::apply::manifest::Manifest;
use crate::apply::types::ApplyRequest;

/// For each document, every earlier document it needs (a custom resource in a new namespace needs
/// both the Namespace and the definition), in buffer order. A document that does not parse needs
/// nothing (its dry run reports why).
pub(super) fn dependencies(
    requests: &[ApplyRequest],
    manifests: &[Option<Manifest>],
) -> Vec<Vec<usize>> {
    (0..manifests.len())
        .map(|index| {
            let Some(me) = manifests[index].as_ref() else {
                return Vec::new();
            };
            (0..index)
                .filter(|&earlier| {
                    manifests[earlier]
                        .as_ref()
                        .is_some_and(|other| needs(me, requests[index].namespace.as_deref(), other))
                })
                .collect()
        })
        .collect()
}

/// Whether `me` (applied with `default_namespace` for a missing namespace) needs `other`.
fn needs(me: &Manifest, default_namespace: Option<&str>, other: &Manifest) -> bool {
    if is_namespace(&other.gvk) {
        let namespace = me.namespace.as_deref().or(default_namespace);
        return namespace == Some(other.name.as_str());
    }
    if is_crd(&other.gvk) {
        let spec = other.object().get("spec");
        let defines = |pointer: &str, want: &str| {
            spec.and_then(|spec| spec.pointer(pointer))
                .and_then(Value::as_str)
                == Some(want)
        };
        return defines("/group", &me.gvk.group) && defines("/names/kind", &me.gvk.kind);
    }
    false
}

fn is_namespace(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "Namespace"
}

fn is_crd(gvk: &Gvk) -> bool {
    &*gvk.group == "apiextensions.k8s.io" && &*gvk.kind == "CustomResourceDefinition"
}
