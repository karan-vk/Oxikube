//! ServiceAccount panel tests: the facts and bound roles as data (`model`), and the panel drawn
//! over fixed data in a test window (`panel`).

mod model;
mod panel;

use oxikube_app::access::SubjectIndex;
use oxikube_app::store::{RowChange, StoreObject};
use oxikube_domain::Resource;
use oxikube_domain::rbac::RbacKind;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn resource(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

pub(super) fn service_account(value: Value) -> Resource {
    let mut object = json!({
        "apiVersion": "v1", "kind": "ServiceAccount",
        "metadata": {"name": "ci", "namespace": "shop"},
    });
    for (key, field) in value.as_object().expect("an object of fields") {
        object[key] = field.clone();
    }
    resource(object)
}

pub(super) fn cluster_role(name: &str) -> Resource {
    resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": name},
        "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}],
    }))
}

pub(super) fn binding(
    namespace: Option<&str>,
    name: &str,
    role: &str,
    subjects: Value,
) -> Resource {
    let mut metadata = json!({"name": name});
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }
    resource(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": if namespace.is_some() { "RoleBinding" } else { "ClusterRoleBinding" },
        "metadata": metadata,
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": role},
        "subjects": subjects,
    }))
}

/// An index holding `objects`, one snapshot per kind.
pub(super) fn index_of(objects: Vec<Resource>) -> SubjectIndex {
    let mut index = SubjectIndex::new();
    for kind in RbacKind::ALL {
        let mut rows: Vec<Arc<StoreObject>> = objects
            .iter()
            .filter(|object| RbacKind::of(&object.kind) == Some(kind))
            .map(|object| Arc::new(StoreObject::Resource(object.clone())))
            .collect();
        rows.sort_by(|a, b| a.name().cmp(b.name()));
        index.apply_rows(kind, &RowChange::Snapshot(rows));
    }
    index
}

pub(super) fn cluster() -> oxikube_domain::ids::ClusterId {
    oxikube_domain::ids::ClusterId::new(
        "test.yaml",
        &oxikube_domain::ids::ContextName::new("kind-a"),
    )
}
