//! Re-checking the index right after an `invalidate`.
//!
//! The API server publishes an edited CRD's document a fraction of a second after the CRD
//! watch reports the edit, so the index read straight after the invalidate can still list the
//! old hash, and the old document would then be cached as an ordinary hit. For
//! [`OpenApiConfig::settle_after_invalidate`](super::OpenApiConfig) after the invalidate,
//! lookups re-read the (small) index now and then and drop whatever the new one changed.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use super::config::RECHECK_TIMEOUT;
use super::fetch::fetch_index;
use super::index::Index;
use super::service::{Loaded, Memory, OpenApiSchemas};

impl OpenApiSchemas {
    /// While the cache is unsettled (see the module docs), re-reads the index when the one in
    /// memory is at least `recheck_every` old and drops the schemas and documents of every
    /// group-version it changed. A failed read keeps the cache and is tried again after
    /// `recheck_every`, not on every lookup in between.
    ///
    /// A change also bumps the epoch, so a lookup that was already running (holding the old
    /// index) cannot cache what it loads after the purge.
    pub(super) async fn recheck_unsettled_index(&self) {
        let Some((epoch, previous)) = self.index_to_recheck() else {
            return;
        };
        let _gate = self.index_gate.lock().await;
        {
            let memory = self.memory.lock();
            let current = memory.loaded.as_ref();
            // Another caller re-read it (or tried and failed, for the callers that queued
            // behind a slow read), or an invalidate cleared it, while this one waited.
            if memory.epoch != epoch
                || !current.is_some_and(|l| Arc::ptr_eq(l, &previous))
                || !self.recheck_due(&memory, &previous)
            {
                return;
            }
        }
        let timeout = self.config.request_timeout.min(RECHECK_TIMEOUT);
        let Ok(index) = fetch_index(&self.client, timeout).await else {
            let mut memory = self.memory.lock();
            if memory.epoch == epoch {
                memory.recheck_failed_at = Some(Instant::now());
            }
            return;
        };
        let changed: HashSet<String> = previous
            .index
            .entries
            .keys()
            .chain(index.entries.keys())
            .filter(|key| previous.index.entries.get(*key) != index.entries.get(*key))
            .cloned()
            .collect();
        let fresh = Arc::new(Loaded {
            index,
            server_version: previous.server_version.clone(),
            at: Instant::now(),
        });
        let mut memory = self.memory.lock();
        if memory.epoch != epoch {
            return;
        }
        let stale = |gvk: &oxikube_domain::ids::Gvk| {
            changed.contains(&Index::key_for(&gvk.group, &gvk.version))
        };
        memory.schemas.retain(|gvk, _| !stale(gvk));
        memory.misses.retain(|gvk, _| !stale(gvk));
        memory.documents.retain(|key, _| !changed.contains(key));
        if !changed.is_empty() {
            // Lookups that started before this hold the old index and may still be fetching
            // its documents: they must not cache them over the purge.
            memory.epoch += 1;
        }
        memory.recheck_failed_at = None;
        memory.loaded = Some(fresh);
    }

    /// The index to compare with a fresh one, when the cache is unsettled and it is due.
    fn index_to_recheck(&self) -> Option<(u64, Arc<Loaded>)> {
        let mut memory = self.memory.lock();
        let since = memory.unsettled_since?;
        // No index yet: the next load is the one that will be compared with nothing.
        let loaded = memory.loaded.clone()?;
        if loaded.at >= since + self.config.settle_after_invalidate {
            // An index read after the settling time: nothing older can be hiding in the cache.
            memory.unsettled_since = None;
            return None;
        }
        self.recheck_due(&memory, &loaded)
            .then(|| (memory.epoch, loaded))
    }

    /// Whether `loaded` and the last failed attempt are both at least `recheck_every` old.
    fn recheck_due(&self, memory: &Memory, loaded: &Loaded) -> bool {
        let every = self.config.recheck_every;
        loaded.at.elapsed() >= every
            && memory
                .recheck_failed_at
                .is_none_or(|failed| failed.elapsed() >= every)
    }
}
