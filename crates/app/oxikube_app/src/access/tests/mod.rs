//! Access tests: the rules service against `FakeAccessReviewPort` and the virtual clock, and the
//! subject index (queries and scopes in `index`, store deltas in `incremental`, recorded
//! `kubectl auth can-i --list` fixtures in `fixtures`, random policies in `random`).

mod accessible;
mod batch;
mod cache;
mod fixtures;
mod incremental;
mod index;
mod random;
mod registry;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::Resource;
use oxikube_domain::access::{AccessCheck, AccessRule, AccessRules};
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::RbacKind;
use oxikube_testkit::{FakeAccessReviewPort, FakeClockPort};
use serde_json::{Value, json};

use super::{AccessService, DEFAULT_TTL, SubjectIndex};
use crate::store::{ObjectKey, RowChange, StoreObject};

fn service(
    port: FakeAccessReviewPort,
) -> (AccessService, Arc<FakeAccessReviewPort>, Arc<FakeClockPort>) {
    let port = Arc::new(port);
    let clock = Arc::new(FakeClockPort::default());
    let service = AccessService::new(port.clone(), clock.clone(), DEFAULT_TTL);
    (service, port, clock)
}

fn reads(resources: &[&str]) -> AccessRules {
    AccessRules::none().with_rule(AccessRule::granting(
        &["get", "list"],
        &[""],
        resources,
        &[],
    ))
}

fn pods() -> Gvr {
    Gvr::new("", "v1", "pods")
}

fn check(verb: &str, namespace: &str) -> AccessCheck {
    AccessCheck::new(verb, pods()).namespace(namespace)
}

const PAST_TTL: Duration = Duration::from_secs(61);

/// A Role (`namespace: Some`) or ClusterRole (`None`) with `rules` and `labels`.
fn role(namespace: Option<&str>, name: &str, rules: Value, labels: Value) -> Resource {
    let mut metadata = json!({"name": name, "labels": labels});
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": if namespace.is_some() { "Role" } else { "ClusterRole" },
        "metadata": metadata,
        "rules": rules,
    }))
    .expect("a valid role")
}

/// A RoleBinding (`namespace: Some`) or ClusterRoleBinding (`None`) of `role_kind`/`role_name`
/// to `subjects`.
fn binding(
    namespace: Option<&str>,
    name: &str,
    (role_kind, role_name): (&str, &str),
    subjects: Value,
) -> Resource {
    let mut metadata = json!({"name": name});
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": if namespace.is_some() { "RoleBinding" } else { "ClusterRoleBinding" },
        "metadata": metadata,
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": role_kind, "name": role_name},
        "subjects": subjects,
    }))
    .expect("a valid binding")
}

/// `objects` grouped per RBAC kind, each kind sorted the way a store subscription lists it.
fn by_kind(objects: impl IntoIterator<Item = Resource>) -> BTreeMap<RbacKind, Vec<Resource>> {
    let mut grouped: BTreeMap<RbacKind, Vec<Resource>> = BTreeMap::new();
    for object in objects {
        let kind = RbacKind::of(&object.kind).expect("an RBAC object");
        grouped.entry(kind).or_default().push(object);
    }
    for list in grouped.values_mut() {
        list.sort_by_key(|object| ObjectKey::of(&object.meta));
    }
    grouped
}

/// The subscription snapshot of `objects` (already sorted).
fn snapshot(objects: &[Resource]) -> RowChange {
    RowChange::Snapshot(
        objects
            .iter()
            .map(|object| Arc::new(StoreObject::Resource(object.clone())))
            .collect(),
    )
}

/// An index holding `objects`, one snapshot per kind.
fn index_of(objects: impl IntoIterator<Item = Resource>) -> SubjectIndex {
    let mut index = SubjectIndex::new();
    for (kind, list) in by_kind(objects) {
        index.apply_rows(kind, &snapshot(&list));
    }
    index
}
