//! [`object_label`]: how an object is named in a message.

use oxikube_domain::ids::ResourceRef;

/// "Pod default/web-0" for a namespaced object, "Namespace payments" for a cluster-scoped one.
pub fn object_label(target: &ResourceRef) -> String {
    match target.namespace() {
        Some(ns) => format!("{} {ns}/{}", target.gvk.kind, target.name),
        None => format!("{} {}", target.gvk.kind, target.name),
    }
}
