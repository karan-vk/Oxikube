//! Admission: how [`StoreInner`] asks the [`FeedBudget`](super::FeedBudget) for a feed, evicts
//! idle (grace-period) feeds while it refuses, asks again for an entry it refused, and releases
//! a slot when a feed is torn down.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ErrorKind;

use super::budget::{Admission, FeedRequest};
use super::delta::FeedState;
use super::entry::FeedEntry;
use super::object::FeedKey;
use super::service::StoreInner;
use super::spawn::TaskGuard;

/// The driver and grace-timer guards of a removed entry; dropping them aborts the tasks.
pub(super) type Retired = (Option<TaskGuard>, Option<TaskGuard>);

impl StoreInner {
    /// A new entry, after asking the budget (evicting idle feeds while it refuses).
    pub(super) fn create(
        &self,
        key: &FeedKey,
        entries: &mut HashMap<FeedKey, Arc<FeedEntry>>,
        evicted: &mut Vec<Retired>,
    ) -> Arc<FeedEntry> {
        let plan = self.plan(&key.gvk);
        let request = FeedRequest {
            key: key.clone(),
            kind: plan.kind,
            priority: plan.priority,
        };
        Arc::new(match self.admit(request.clone(), entries, evicted) {
            Ok(granted) => FeedEntry::new(key.clone(), granted, true, FeedState::Warming),
            Err(state) => FeedEntry::new(key.clone(), request, false, state),
        })
    }

    /// Asks the budget again for an entry it refused (another view subscribed, and feeds may
    /// have closed since); on a grant the caller starts its driver.
    pub(super) fn readmit(
        &self,
        entry: &FeedEntry,
        entries: &mut HashMap<FeedKey, Arc<FeedEntry>>,
        evicted: &mut Vec<Retired>,
    ) {
        let request = {
            let st = entry.state.lock();
            if st.admitted {
                return;
            }
            st.request.clone()
        };
        // The entry is not admitted, so the budget does not count it, and it has subscribers,
        // so it is never evicted.
        match self.admit(request, entries, evicted) {
            Ok(granted) => {
                let mut st = entry.state.lock();
                st.request = granted;
                st.admitted = true;
            }
            Err(state) => FeedEntry::publish(&mut entry.state.lock(), &entry.key, state),
        }
    }

    /// Asks the budget for `request` (evicting idle feeds while it refuses): the granted
    /// request, or the `Failed` state to show. The caller holds the entry map and no entry.
    fn admit(
        &self,
        mut request: FeedRequest,
        entries: &mut HashMap<FeedKey, Arc<FeedEntry>>,
        evicted: &mut Vec<Retired>,
    ) -> Result<FeedRequest, FeedState> {
        loop {
            let running = entries.values().filter(|e| e.state.lock().admitted).count();
            match self.options.budget.admit(&request, running) {
                Admission::Granted => return Ok(request),
                Admission::Degraded(kind) => {
                    request.kind = kind;
                    return Ok(request);
                }
                Admission::Refused(reason) => {
                    if let Some(guards) = self.evict_oldest_idle(entries) {
                        evicted.push(guards);
                        continue;
                    }
                    tracing::info!(feed = %request.key, "resource store feed refused by the watch budget");
                    self.options.budget.refused(&request);
                    return Err(FeedState::Failed {
                        kind: ErrorKind::BudgetExceeded,
                        message: reason,
                    });
                }
            }
        }
    }

    /// Removes the idle entry that has been idle longest; returns its guards to drop.
    fn evict_oldest_idle(&self, entries: &mut HashMap<FeedKey, Arc<FeedEntry>>) -> Option<Retired> {
        let oldest = entries
            .values()
            .filter_map(|e| {
                let st = e.state.lock();
                (st.subscribers.is_empty() && st.admitted)
                    .then(|| (st.idle_since.unwrap_or(Timestamp::MIN), e.key.clone()))
            })
            .min()?;
        let entry = entries.remove(&oldest.1)?;
        tracing::debug!(feed = %entry.key, "resource store evicted an idle feed for the budget");
        Some(self.retire(&entry))
    }

    /// Releases the budget slot and takes the entry's task guards (dropping them aborts).
    pub(super) fn retire(&self, entry: &FeedEntry) -> Retired {
        let mut st = entry.state.lock();
        st.running = false;
        if st.admitted {
            self.options.budget.released(&st.request);
        }
        (st.driver.take(), st.grace.take())
    }
}
