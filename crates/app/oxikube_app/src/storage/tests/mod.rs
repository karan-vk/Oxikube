//! Storage tests over `oxikube_testkit` fakes, through the real `CommandBus` and `MutationGuard`
//! (see [`crate::testing::Harness`]).
//!
//! * `resize`: `pvc::Resize` (growth, expansion, read-only, dry run, the stale size, audit)
//! * `default_class`: `storageclass::SetDefault`
//! * `flows`: the two dialogs' flows
//! * `usage`: the Pods (used by) index and the claims columns
//! * `feed`: the index kept from a store's Pods feed

mod default_class;
mod feed;
mod flows;
mod resize;
mod usage;

use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};

use crate::storage::register_commands;
use crate::testing::{Harness, id};

pub(super) const CLUSTER: &str = "a";

/// A harness with the storage handlers registered, connected writable to cluster `a`.
pub(super) fn harness() -> Harness {
    let h =
        Harness::with_extra(|registry, _, manager| register_commands(registry, manager.clone()));
    h.connect(CLUSTER, false);
    h
}

pub(super) fn claim_ref(namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        id(CLUSTER),
        Gvk::new("", "v1", "PersistentVolumeClaim"),
        namespace,
        name,
    )
}

pub(super) fn class_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(
        id(CLUSTER),
        Gvk::new("storage.k8s.io", "v1", "StorageClass"),
        name,
    )
}

/// Puts `objects` in the cluster's fake store.
pub(super) fn seed(h: &Harness, objects: impl IntoIterator<Item = Resource>) {
    let resources = h.resources(CLUSTER);
    for object in objects {
        resources.insert(object);
    }
}

/// Queues `n` successful patch answers (the rehearsal and the real patch of each write).
pub(super) fn script_patches(h: &Harness, answer: &Resource, n: usize) {
    for _ in 0..n {
        h.resources(CLUSTER).script().patch.push_ok(answer.clone());
    }
}
