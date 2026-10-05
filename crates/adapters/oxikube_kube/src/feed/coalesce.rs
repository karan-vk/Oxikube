//! [`Coalescer`]: folds deltas into the next batch, merging by object.
//!
//! Every delta for an object already in the unsent batch replaces that entry in place
//! (`Applied` then `Applied` keeps the latest state, `Applied` then `Deleted` keeps the
//! delete), so a pod that changes ten times in one window costs one delta, and a consumer
//! that falls behind costs at most one delta per object, never an unbounded queue. Entries
//! are keyed by UID (plus namespace and name), so a deleted object and a re-created namesake
//! stay two deltas in their original order. A `Restarted` discards everything before it.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_ports::{Delta, DeltaBatch};

/// Identity of one object within a batch.
type Key = (Option<Arc<str>>, Arc<str>, Option<Arc<str>>);

fn key_of(resource: &Resource) -> Key {
    let meta = &resource.meta;
    (meta.namespace.clone(), meta.name.clone(), meta.uid.clone())
}

/// The batch being built. Its buffers are reused across batches.
#[derive(Default)]
pub(super) struct Coalescer {
    deltas: Vec<Delta<Resource>>,
    index: HashMap<Key, usize>,
    resource_version: Option<Arc<str>>,
}

impl Coalescer {
    /// Folds `delta` into the batch.
    pub(super) fn push(&mut self, delta: Delta<Resource>) {
        let resource = match &delta {
            Delta::Restarted(_) => {
                self.deltas.clear();
                self.index.clear();
                self.deltas.push(delta);
                return;
            }
            Delta::Applied(r) | Delta::Deleted(r) => r,
        };
        if let Some(rv) = &resource.meta.resource_version {
            self.resource_version = Some(rv.clone());
        }
        match self.index.entry(key_of(resource)) {
            Entry::Occupied(slot) => self.deltas[*slot.get()] = delta,
            Entry::Vacant(slot) => {
                slot.insert(self.deltas.len());
                self.deltas.push(delta);
            }
        }
    }

    /// Puts a `Restarted` with `objects` in front of what is already queued: the deltas
    /// queued so far happened after those objects were listed.
    pub(super) fn prepend_restart(&mut self, objects: Vec<Resource>) {
        self.deltas.insert(0, Delta::Restarted(objects));
        for at in self.index.values_mut() {
            *at += 1;
        }
    }

    /// Number of deltas queued.
    pub(super) fn len(&self) -> usize {
        self.deltas.len()
    }

    /// Whether nothing is queued.
    pub(super) fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    /// Takes the queued deltas as one batch, leaving the coalescer empty with its buffers.
    pub(super) fn take(&mut self) -> DeltaBatch<Resource> {
        self.index.clear();
        let capacity = self.deltas.capacity().min(1024);
        let deltas = std::mem::replace(&mut self.deltas, Vec::with_capacity(capacity));
        DeltaBatch {
            deltas,
            resource_version: self.resource_version.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pod(name: &str, uid: &str, rv: &str) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": name, "namespace": "ns", "uid": uid, "resourceVersion": rv},
        }))
        .unwrap()
    }

    fn rv(delta: &Delta<Resource>) -> String {
        match delta {
            Delta::Applied(r) | Delta::Deleted(r) => {
                r.meta.resource_version.as_deref().unwrap().to_owned()
            }
            Delta::Restarted(_) => "restart".into(),
        }
    }

    #[test]
    fn repeated_changes_to_one_object_keep_the_latest() {
        let mut c = Coalescer::default();
        for v in 1..=10 {
            c.push(Delta::Applied(pod("a", "u1", &v.to_string())));
        }
        c.push(Delta::Applied(pod("b", "u2", "11")));
        let batch = c.take();
        assert_eq!(batch.len(), 2);
        assert_eq!(rv(&batch.deltas[0]), "10");
        assert_eq!(batch.resource_version.as_deref(), Some("11"));
        assert!(c.is_empty());
    }

    #[test]
    fn a_delete_replaces_a_pending_apply_in_place() {
        let mut c = Coalescer::default();
        c.push(Delta::Applied(pod("a", "u1", "1")));
        c.push(Delta::Applied(pod("b", "u2", "2")));
        c.push(Delta::Deleted(pod("a", "u1", "3")));
        let batch = c.take();
        assert!(matches!(&batch.deltas[0], Delta::Deleted(r) if r.name() == "a"));
        assert!(matches!(&batch.deltas[1], Delta::Applied(r) if r.name() == "b"));
    }

    #[test]
    fn a_recreated_namesake_stays_after_the_delete() {
        let mut c = Coalescer::default();
        c.push(Delta::Deleted(pod("a", "u1", "1")));
        c.push(Delta::Applied(pod("a", "u2", "2")));
        c.push(Delta::Applied(pod("a", "u2", "3")));
        let batch = c.take();
        assert_eq!(batch.len(), 2);
        assert!(matches!(&batch.deltas[0], Delta::Deleted(_)));
        assert_eq!(rv(&batch.deltas[1]), "3");
    }

    #[test]
    fn restarted_discards_what_came_before() {
        let mut c = Coalescer::default();
        c.push(Delta::Applied(pod("a", "u1", "1")));
        c.push(Delta::Restarted(vec![pod("b", "u2", "2")]));
        c.push(Delta::Applied(pod("a", "u1", "3")));
        let batch = c.take();
        assert_eq!(batch.len(), 2);
        assert!(batch.deltas[0].is_restart());
        assert_eq!(rv(&batch.deltas[1]), "3");
    }

    #[test]
    fn prepended_restart_goes_first_and_merging_still_works() {
        let mut c = Coalescer::default();
        c.push(Delta::Applied(pod("a", "u1", "5")));
        c.prepend_restart(vec![pod("a", "u1", "4")]);
        c.push(Delta::Applied(pod("a", "u1", "6")));
        let batch = c.take();
        assert_eq!(batch.len(), 2);
        assert!(batch.deltas[0].is_restart());
        assert_eq!(rv(&batch.deltas[1]), "6");
    }
}
