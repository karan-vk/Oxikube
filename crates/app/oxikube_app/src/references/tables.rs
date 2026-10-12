//! [`Tables`]: the two maps behind the index, a reference to its Pods and a Pod to its
//! references, and the only code that edits them.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use oxikube_domain::view::storage::pod_is_active;
use oxikube_domain::view::{PodRef, RefKind, pod_references};
use oxikube_domain::{ObjectMeta, OwnerRef, Resource};

use super::answer::PodKey;
use crate::store::StoreObject;

/// The Pods that use one object.
pub(super) type Pods = BTreeSet<PodKey>;
/// Object name to its Pods.
type ByName = HashMap<Arc<str>, Pods>;
/// Namespace (empty for a cluster-scoped kind) to its objects.
type ByNamespace = HashMap<Arc<str>, ByName>;

/// What the index keeps of one Pod: everything it needs to take the Pod's references out again
/// without the Pod's JSON.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct PodEntry {
    refs: Vec<PodRef>,
    pub(super) owner: Option<OwnerRef>,
}

/// What a Pod should be indexed as: its references and owner, built outside the lock.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Desired {
    refs: Vec<PodRef>,
    owner: Option<OwnerRef>,
}

impl Desired {
    /// What `pod` points at; `None` when it is not indexed at all: not a Pod resource (a Table
    /// row), a finished one (it no longer holds what it mounted), or one that names nothing.
    pub(super) fn of(object: &StoreObject) -> Option<Self> {
        let pod = object.resource()?;
        Self::of_pod(pod)
    }

    fn of_pod(pod: &Resource) -> Option<Self> {
        if !pod_is_active(pod) {
            return None;
        }
        let refs = pod_references(pod);
        if refs.is_empty() {
            return None;
        }
        let owner = primary_owner(&pod.meta).cloned();
        Some(Self { refs, owner })
    }
}

/// The controller of `meta` (else its first owner): the one a workload is found from.
pub(super) fn primary_owner(meta: &ObjectMeta) -> Option<&OwnerRef> {
    meta.controller_ref().or_else(|| meta.owner_refs.first())
}

/// The identity of `object` in the index.
pub(super) fn key_of(object: &StoreObject) -> PodKey {
    let meta = object.meta();
    PodKey {
        namespace: meta.namespace.clone().unwrap_or_else(|| Arc::from("")),
        name: meta.name.clone(),
    }
}

/// The index's data. All edits return whether an answer may have changed.
#[derive(Default)]
pub(super) struct Tables {
    refs: [ByNamespace; RefKind::ALL.len()],
    pods: HashMap<PodKey, PodEntry>,
    /// The key of cluster-scoped objects' namespace level.
    empty: Option<Arc<str>>,
}

impl Tables {
    /// How many Pods are indexed (those that name something and are active).
    pub(super) fn indexed(&self) -> usize {
        self.pods.len()
    }

    /// The Pods that use `name` of `kind` in `namespace` (ignored for a cluster-scoped kind).
    pub(super) fn users(
        &self,
        kind: RefKind,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<&Pods> {
        let namespace = if kind.is_cluster_scoped() {
            ""
        } else {
            namespace?
        };
        self.refs[kind.slot()].get(namespace)?.get(name)
    }

    /// The whole level of `kind`, for the tests' dump of the index.
    #[cfg(test)]
    pub(super) fn level(&self, kind: RefKind) -> &HashMap<Arc<str>, HashMap<Arc<str>, Pods>> {
        &self.refs[kind.slot()]
    }

    /// The indexed entry of `key`.
    pub(super) fn entry(&self, key: &PodKey) -> Option<&PodEntry> {
        self.pods.get(key)
    }

    /// Makes `key` indexed as `desired` (or not at all for `None`), replacing what was there.
    /// A Pod whose references and owner did not change costs a comparison and no edit, the usual
    /// case for a status update.
    pub(super) fn set(&mut self, key: &PodKey, desired: Option<Desired>) -> bool {
        match (self.pods.get(key), &desired) {
            (None, None) => return false,
            (Some(old), Some(new)) if old.refs == new.refs && old.owner == new.owner => {
                return false;
            }
            _ => {}
        }
        self.remove(key);
        if let Some(desired) = desired {
            self.insert(key, desired);
        }
        true
    }

    fn insert(&mut self, key: &PodKey, desired: Desired) {
        let empty = self.empty.get_or_insert_with(|| Arc::from("")).clone();
        let mut refs = Vec::with_capacity(desired.refs.len());
        for PodRef { kind, name } in desired.refs {
            let namespace = if kind.is_cluster_scoped() {
                empty.clone()
            } else {
                key.namespace.clone()
            };
            let by_name = self.refs[kind.slot()].entry(namespace).or_default();
            // One allocation per distinct name, however many Pods use it.
            let name = by_name
                .get_key_value(&*name)
                .map_or(name, |(known, _)| known.clone());
            by_name.entry(name.clone()).or_default().insert(key.clone());
            refs.push(PodRef { kind, name });
        }
        self.pods.insert(
            key.clone(),
            PodEntry {
                refs,
                owner: desired.owner,
            },
        );
    }

    fn remove(&mut self, key: &PodKey) {
        let Some(entry) = self.pods.remove(key) else {
            return;
        };
        for PodRef { kind, name } in &entry.refs {
            let namespace: &str = if kind.is_cluster_scoped() {
                ""
            } else {
                &key.namespace
            };
            let level = &mut self.refs[kind.slot()];
            let Some(by_name) = level.get_mut(namespace) else {
                continue;
            };
            if let Some(pods) = by_name.get_mut(&**name) {
                pods.remove(key);
                if pods.is_empty() {
                    by_name.remove(&**name);
                }
            }
            if by_name.is_empty() {
                level.remove(namespace);
            }
        }
    }
}
