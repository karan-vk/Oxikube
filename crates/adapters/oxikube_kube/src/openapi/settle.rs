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

use super::fetch::fetch_index;
use super::index::Index;
use super::service::{Loaded, OpenApiSchemas};

impl OpenApiSchemas {
    /// While the cache is unsettled (see the module docs), re-reads the index when the one in
    /// memory is at least `recheck_every` old and drops the schemas and documents of every
    /// group-version it changed. A failed read keeps the cache and is tried again later.
    pub(super) async fn recheck_unsettled_index(&self) {
        let Some((epoch, previous)) = self.index_to_recheck() else {
            return;
        };
        let _gate = self.index_gate.lock().await;
        let current = self.memory.lock().loaded.clone();
        // Another caller re-read it, or an invalidate cleared it, while this one waited.
        if !current.is_some_and(|l| Arc::ptr_eq(&l, &previous)) {
            return;
        }
        let Ok(index) = fetch_index(&self.client, self.config.request_timeout).await else {
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
        (loaded.at.elapsed() >= self.config.recheck_every).then(|| (memory.epoch, loaded))
    }
}
