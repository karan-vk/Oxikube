//! [`SummaryMemo`]: each pod's [`PodSummary`], derived once per object version and shared by
//! every cell of its row.
//!
//! A row has ten computed cells (Ready, Status, Restarts, the four `%` columns, Service Account,
//! Controlled By, QoS...) and the store's sort, the table's cell cache (which starts over every
//! second, as ages move) and the once-a-second age check all ask for them. Without the memo each
//! of those asks would walk the pod's JSON again; with it, a pod is summarised once when a new
//! version of it is first read (on the store's background spawner when the table sorts by a pod
//! column, else on its first draw) and every later read is a hash lookup.
//!
//! An entry is keyed by the object's namespace and name and remembers the JSON document it was
//! derived from (a clone of the shared bytes, so that allocation cannot be reused while the entry
//! holds it): a hit needs the very same document ([`JsonDoc::ptr_eq`]), and a new version
//! replaces its predecessor's entry. Deleted pods leave entries behind until the memo outgrows
//! [`MAX_ENTRIES`] and starts over (the next reads derive again).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::json::JsonDoc;
use oxikube_domain::{PodSummary, Resource};
use parking_lot::Mutex;

use crate::store::ObjectKey;

/// Entries held at most: two 10 000-pod clusters with churn. Beyond it the memo starts over.
pub(super) const MAX_ENTRIES: usize = 32_768;

struct Entry {
    doc: JsonDoc,
    summary: Arc<PodSummary>,
}

/// See the [module docs](self).
#[derive(Default)]
pub(super) struct SummaryMemo {
    entries: Mutex<HashMap<ObjectKey, Entry>>,
    derived: AtomicU64,
}

impl SummaryMemo {
    /// The summary of `pod`, derived on the first read of this version. `None` for an object
    /// that is not a full Pod (a metadata-only object has no spec or status to summarise).
    pub(super) fn get(&self, pod: &Resource) -> Option<Arc<PodSummary>> {
        if pod.is_partial() {
            return None;
        }
        let key = ObjectKey::of(&pod.meta);
        if let Some(entry) = self.entries.lock().get(&key)
            && entry.doc.ptr_eq(pod.doc())
        {
            return Some(entry.summary.clone());
        }
        // Derived outside the lock: the UI thread and the store's sort may read concurrently.
        let summary = Arc::new(PodSummary::from_resource(pod).ok()?);
        self.derived.fetch_add(1, Ordering::Relaxed);
        let mut entries = self.entries.lock();
        if entries.len() >= MAX_ENTRIES && !entries.contains_key(&key) {
            entries.clear();
        }
        entries.insert(
            key,
            Entry {
                doc: pod.doc().clone(),
                summary: summary.clone(),
            },
        );
        Some(summary)
    }

    /// How many summaries were derived so far (each a walk of one pod's JSON).
    pub(super) fn derived(&self) -> u64 {
        self.derived.load(Ordering::Relaxed)
    }

    /// Entries held now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.lock().len()
    }
}
