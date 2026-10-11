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
//! # Identity
//!
//! An entry is keyed by the pod's `metadata.uid` and remembers the `metadata.resourceVersion` it
//! was derived from: a hit needs the same version, and a new version replaces its predecessor's
//! entry. A UID is a server-generated UUID, so two clusters' pods with the same namespace and
//! name (`monitoring/prometheus-0` in staging and in prod) never share an entry and one provider
//! serves every cluster's table. An entry holds the summary only, never the pod's JSON, so it
//! keeps no document alive. A pod without a UID or a resource version (never one from an API
//! server: a hand-built fixture) is derived on every read and never held.
//!
//! # Release
//!
//! Every [`SWEEP_EVERY`] a read drops the entries nobody read since the previous sweep: deleted
//! pods, a closed table's or a disconnected cluster's pods go within two periods of their last
//! read, and a pod scrolled back into view after that is derived again (once). [`MAX_ENTRIES`]
//! bounds the memo between sweeps.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use oxikube_domain::{PodSummary, Resource};
use parking_lot::Mutex;

/// Entries held at most: two 10 000-pod clusters with churn. Beyond it the memo starts over.
pub(super) const MAX_ENTRIES: usize = 32_768;

/// How often a read sweeps out the entries not read since the previous sweep.
pub(super) const SWEEP_EVERY: Duration = Duration::from_secs(60);

struct Entry {
    /// `metadata.resourceVersion` of the version summarised.
    version: Arc<str>,
    summary: Arc<PodSummary>,
    /// The sweep period the entry was last read in.
    period: u64,
}

struct State {
    /// By `metadata.uid`.
    entries: HashMap<Arc<str>, Entry>,
    /// The current sweep period: bumped by every sweep.
    period: u64,
    swept_at: Instant,
}

impl State {
    /// Drops the entries not read since the previous sweep, once [`SWEEP_EVERY`] has passed.
    fn sweep_if_due(&mut self, now: Instant) {
        if now.saturating_duration_since(self.swept_at) < SWEEP_EVERY {
            return;
        }
        let period = self.period;
        self.entries.retain(|_, entry| entry.period == period);
        self.period += 1;
        self.swept_at = now;
    }
}

/// See the [module docs](self).
pub(super) struct SummaryMemo {
    state: Mutex<State>,
    derived: AtomicU64,
}

impl Default for SummaryMemo {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                period: 0,
                swept_at: Instant::now(),
            }),
            derived: AtomicU64::new(0),
        }
    }
}

impl SummaryMemo {
    /// The summary of `pod`, derived on the first read of this version. `None` for an object
    /// that is not a full Pod (a metadata-only object has no spec or status to summarise).
    pub(super) fn get(&self, pod: &Resource) -> Option<Arc<PodSummary>> {
        self.get_at(pod, Instant::now())
    }

    /// [`get`](Self::get) as of `now` (the sweep clock).
    pub(super) fn get_at(&self, pod: &Resource, now: Instant) -> Option<Arc<PodSummary>> {
        if pod.is_partial() {
            return None;
        }
        let (Some(uid), Some(version)) = (&pod.meta.uid, &pod.meta.resource_version) else {
            return self.derive(pod);
        };
        {
            let mut state = self.state.lock();
            state.sweep_if_due(now);
            let period = state.period;
            if let Some(entry) = state.entries.get_mut(uid)
                && entry.version == *version
            {
                entry.period = period;
                return Some(entry.summary.clone());
            }
        }
        // Derived outside the lock: the UI thread and the store's sort may read concurrently.
        let summary = self.derive(pod)?;
        let mut state = self.state.lock();
        if state.entries.len() >= MAX_ENTRIES && !state.entries.contains_key(uid) {
            state.entries.clear();
        }
        let period = state.period;
        state.entries.insert(
            uid.clone(),
            Entry {
                version: version.clone(),
                summary: summary.clone(),
                period,
            },
        );
        Some(summary)
    }

    fn derive(&self, pod: &Resource) -> Option<Arc<PodSummary>> {
        let summary = PodSummary::from_resource(pod).ok()?;
        self.derived.fetch_add(1, Ordering::Relaxed);
        Some(Arc::new(summary))
    }

    /// How many summaries were derived so far (each a walk of one pod's JSON).
    pub(super) fn derived(&self) -> u64 {
        self.derived.load(Ordering::Relaxed)
    }

    /// Entries held now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.state.lock().entries.len()
    }
}
