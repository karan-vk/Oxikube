//! [`ObjectCache`]: one feed's objects keyed by [`ObjectKey`], with secondary indices by name,
//! namespace and label, and the delta application that turns a [`FeedBatch`] into a
//! [`CacheChange`] (the keys whose visible state moved).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::counts::{KindCount, Tally};
use super::feed::{FeedBatch, ObjectDelta};
use super::object::{FeedScope, ObjectKey, StoreObject};
use super::query::StoreFilter;

type KeySet = HashSet<ObjectKey>;

/// What one applied batch changed: the final state of every key it touched.
#[derive(Debug, Default)]
pub(crate) struct CacheChange {
    /// Keys added or modified, with their new object.
    pub upserted: Vec<Arc<StoreObject>>,
    /// Keys deleted.
    pub removed: Vec<ObjectKey>,
    /// Whether the batch held a relist.
    pub restarted: bool,
}

impl CacheChange {
    pub fn len(&self) -> usize {
        self.upserted.len() + self.removed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Objects plus indices. Applying deltas is tolerant: a delete of an unknown key is ignored and
/// a modify of an unknown key inserts it, so out-of-order or duplicated events converge.
#[derive(Debug, Default)]
pub(crate) struct ObjectCache {
    objects: HashMap<ObjectKey, Arc<StoreObject>>,
    /// How many objects have a health verdict, and how many of those are healthy: kept in step
    /// by `upsert` and `remove`, so a count read is O(1) however many objects there are.
    tally: Tally,
    by_name: HashMap<Arc<str>, KeySet>,
    by_namespace: HashMap<Arc<str>, KeySet>,
    by_label: HashMap<(Arc<str>, Arc<str>), KeySet>,
}

impl ObjectCache {
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// The health tally of the objects whose namespace `scope` covers. O(1) for a cluster-wide
    /// part (the common case); a namespace part of a wider feed walks the objects.
    pub fn tally_in(&self, scope: &FeedScope) -> KindCount {
        let count = |tally: Tally, total| KindCount {
            total,
            rated: tally.rated,
            healthy: tally.healthy,
        };
        match scope {
            FeedScope::Cluster => count(self.tally, self.objects.len()),
            FeedScope::Namespace(_) => {
                let mut tally = Tally::default();
                let mut total = 0;
                for object in self
                    .objects
                    .values()
                    .filter(|o| scope.covers(o.namespace()))
                {
                    total += 1;
                    tally.add(object);
                }
                count(tally, total)
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<StoreObject>> {
        self.objects.values()
    }

    /// Applies `batch` in order and reports the net change.
    pub fn apply(&mut self, batch: FeedBatch) -> CacheChange {
        let mut touched: HashSet<ObjectKey> = HashSet::new();
        let mut restarted = false;
        for delta in batch.deltas {
            match delta {
                ObjectDelta::Applied(object) => {
                    let key = object.key();
                    if self.upsert(key.clone(), object) {
                        touched.insert(key);
                    }
                }
                ObjectDelta::Deleted(key) => {
                    if self.remove(&key).is_some() {
                        touched.insert(key);
                    }
                }
                ObjectDelta::Restarted(objects) => {
                    restarted = true;
                    self.restart(objects, &mut touched);
                }
            }
        }
        let mut change = CacheChange {
            restarted,
            ..CacheChange::default()
        };
        for key in touched {
            match self.objects.get(&key) {
                Some(object) => change.upserted.push(object.clone()),
                None => change.removed.push(key),
            }
        }
        change
    }

    /// Replaces everything with `objects`, touching only the keys whose version changed.
    fn restart(&mut self, objects: Vec<Arc<StoreObject>>, touched: &mut HashSet<ObjectKey>) {
        let fresh: HashSet<ObjectKey> = objects.iter().map(|o| o.key()).collect();
        let gone: Vec<ObjectKey> = self
            .objects
            .keys()
            .filter(|k| !fresh.contains(*k))
            .cloned()
            .collect();
        for key in gone {
            self.remove(&key);
            touched.insert(key);
        }
        for object in objects {
            let key = object.key();
            if self.upsert(key.clone(), object) {
                touched.insert(key);
            }
        }
    }

    /// Inserts or replaces; returns whether anything visible changed.
    pub fn upsert(&mut self, key: ObjectKey, object: Arc<StoreObject>) -> bool {
        if let Some(old) = self.objects.get(&key) {
            if old.same_version(&object) {
                return false;
            }
            let old = old.clone();
            self.unindex(&key, &old);
            self.tally.sub(&old);
        }
        self.index(&key, &object);
        self.tally.add(&object);
        self.objects.insert(key, object);
        true
    }

    pub fn remove(&mut self, key: &ObjectKey) -> Option<Arc<StoreObject>> {
        let old = self.objects.remove(key)?;
        self.unindex(key, &old);
        self.tally.sub(&old);
        Some(old)
    }

    fn index(&mut self, key: &ObjectKey, object: &StoreObject) {
        let meta = object.meta();
        self.by_name
            .entry(meta.name.clone())
            .or_default()
            .insert(key.clone());
        if let Some(ns) = &meta.namespace {
            self.by_namespace
                .entry(ns.clone())
                .or_default()
                .insert(key.clone());
        }
        for (k, v) in &meta.labels {
            self.by_label
                .entry((k.clone(), v.clone()))
                .or_default()
                .insert(key.clone());
        }
    }

    fn unindex(&mut self, key: &ObjectKey, object: &StoreObject) {
        let meta = object.meta();
        index_remove(&mut self.by_name, &meta.name, key);
        if let Some(ns) = &meta.namespace {
            index_remove(&mut self.by_namespace, ns, key);
        }
        for (k, v) in &meta.labels {
            index_remove(&mut self.by_label, &(k.clone(), v.clone()), key);
        }
    }

    /// The objects that pass `filter`, starting from the smallest index set the filter allows
    /// (exact name, a label equality, the namespaces) instead of scanning every object.
    pub fn matching<'a>(&'a self, filter: &'a StoreFilter) -> Vec<&'a Arc<StoreObject>> {
        let mut best: Option<Vec<&ObjectKey>> = None;
        let mut consider = |keys: Vec<&'a ObjectKey>| {
            if best.as_ref().is_none_or(|b| keys.len() < b.len()) {
                best = Some(keys);
            }
        };
        if let Some(name) = &filter.name {
            consider(
                self.by_name
                    .get(name.as_str())
                    .into_iter()
                    .flatten()
                    .collect(),
            );
        }
        if let Some(labels) = &filter.labels {
            for (k, v) in labels.equalities() {
                let at = (Arc::<str>::from(k), Arc::<str>::from(v));
                consider(self.by_label.get(&at).into_iter().flatten().collect());
            }
        }
        if let Some(namespaces) = &filter.namespaces {
            consider(
                namespaces
                    .iter()
                    .filter_map(|ns| self.by_namespace.get(ns.as_str()))
                    .flatten()
                    .collect(),
            );
        }
        match best {
            Some(keys) => keys
                .into_iter()
                .filter_map(|k| self.objects.get(k))
                .filter(|o| filter.matches(o))
                .collect(),
            None => self
                .objects
                .values()
                .filter(|o| filter.matches(o))
                .collect(),
        }
    }
}

fn index_remove<K: std::hash::Hash + Eq>(index: &mut HashMap<K, KeySet>, at: &K, key: &ObjectKey) {
    if let Some(set) = index.get_mut(at) {
        set.remove(key);
        if set.is_empty() {
            index.remove(at);
        }
    }
}
