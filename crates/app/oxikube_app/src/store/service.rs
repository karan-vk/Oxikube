//! [`ResourceStore`]: one cluster session's cache of feeds, keyed by (gvk, scope), with
//! ref-counted subscriptions, grace-period teardown and the budget hook.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use jiff::Timestamp;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, Gvk};
use parking_lot::Mutex;

use super::budget::{Admission, FeedRequest};
use super::config::{FeedInfo, StoreOptions, StoreRuntime};
use super::delta::FeedState;
use super::driver::{Backoff, drive};
use super::entry::{FeedEntry, SubId};
use super::feed::StorePorts;
use super::object::{FeedKey, FeedScope};
use super::policy::FeedPlan;
use super::query::StoreQuery;
use super::spawn::{TaskGuard, spawn_guarded};
use super::subscription::{SubShared, Subscription};
use crate::session::ClusterSession;

/// The app-side cache over one cluster session's feeds (ADR 0006): every table, sidebar count
/// and overview tile subscribes here and never learns which feed served it.
///
/// Cheap to clone (shared state). Plain async Rust: no gpui, no kube. See the
/// [module docs](super) for the full picture.
#[derive(Clone)]
pub struct ResourceStore {
    inner: Arc<StoreInner>,
}

impl std::fmt::Debug for ResourceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceStore")
            .field("cluster", &self.inner.cluster)
            .field("feeds", &self.inner.entries.lock().len())
            .finish_non_exhaustive()
    }
}

impl ResourceStore {
    /// A store over `ports` for `cluster`.
    pub fn new(
        cluster: ClusterId,
        ports: StorePorts,
        runtime: StoreRuntime,
        options: StoreOptions,
    ) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                cluster,
                ports,
                runtime,
                options,
                entries: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// A store over a connected session's ports; `None` while the session is not connected.
    pub fn for_session(
        session: &ClusterSession,
        runtime: StoreRuntime,
        options: StoreOptions,
    ) -> Option<Self> {
        let ports = StorePorts {
            resources: session.resources()?,
            tables: session.tables()?,
        };
        Some(Self::new(session.id().clone(), ports, runtime, options))
    }

    /// The cluster this store caches.
    pub fn cluster(&self) -> &ClusterId {
        &self.inner.cluster
    }

    /// The ports the store reads (to tell whether a session reconnected under it).
    pub(crate) fn ports(&self) -> &StorePorts {
        &self.inner.ports
    }

    /// The feed the policy picks for `gvk`.
    pub fn plan(&self, gvk: &Gvk) -> FeedPlan {
        self.inner.plan(gvk)
    }

    /// Subscribes to `query`: attaches to (or starts) one shared feed per part of its scope and
    /// returns the stream of [`StoreDelta`](super::StoreDelta)s. Never blocks: feeds start on the
    /// store's spawner.
    pub fn subscribe(&self, query: StoreQuery) -> Subscription {
        Subscription::open(self.inner.clone(), query)
    }

    /// Every cached feed, sorted by key.
    pub fn feeds(&self) -> Vec<FeedInfo> {
        let entries = self.inner.entries.lock();
        let mut out: Vec<FeedInfo> = entries
            .values()
            .map(|e| {
                let st = e.state.lock();
                FeedInfo {
                    key: e.key.clone(),
                    kind: e.request.kind,
                    subscribers: st.subscribers.len(),
                    objects: st.cache.len(),
                    state: st.feed_state.clone(),
                    idle: st.subscribers.is_empty(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }
}

/// The shared state behind [`ResourceStore`] and its subscriptions.
pub(crate) struct StoreInner {
    cluster: ClusterId,
    ports: StorePorts,
    runtime: StoreRuntime,
    options: StoreOptions,
    entries: Mutex<HashMap<FeedKey, Arc<FeedEntry>>>,
    next_id: AtomicU64,
}

impl StoreInner {
    pub fn next_id(&self) -> SubId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn plan(&self, gvk: &Gvk) -> FeedPlan {
        self.options.policy.plan(gvk)
    }

    /// Adds subscriber `id` to the entry for (`gvk`, `part`), creating and starting it when
    /// needed (a new entry is seeded from `seed_from`'s objects in `part`), and seeds the
    /// subscriber from it.
    pub fn attach(
        self: &Arc<Self>,
        gvk: &Gvk,
        part: &FeedScope,
        id: SubId,
        sub: &Arc<SubShared>,
        seed_from: &[Arc<FeedEntry>],
    ) -> Arc<FeedEntry> {
        let key = FeedKey {
            gvk: gvk.clone(),
            scope: part.clone(),
        };
        let mut evicted = Vec::new();
        let mut entries = self.entries.lock();
        let created = !entries.contains_key(&key);
        if created {
            let entry = self.create(&key, &mut entries, &mut evicted);
            entries.insert(key.clone(), entry);
        }
        let entry = entries[&key].clone();
        let mut st = entry.state.lock();
        if created {
            for source in seed_from {
                let source = source.state.lock();
                for object in source.cache.values() {
                    if part.covers(object.namespace()) {
                        st.cache.upsert(object.key(), object.clone());
                    }
                }
            }
        }
        st.subscribers.push((id, sub.clone()));
        st.grace = None;
        st.idle_since = None;
        if entry.admitted && !st.running {
            st.running = true;
            // A restart after a terminal error: subscribers already attached see it warm up.
            FeedEntry::publish(&mut st, &entry.key, FeedState::Warming);
            let task = drive(
                Arc::downgrade(&entry),
                self.ports.clone(),
                entry.request.kind,
                key,
                self.runtime.clock.clone(),
                Backoff::new(
                    self.options.config.retry_initial,
                    self.options.config.retry_max,
                ),
            );
            st.driver = Some(spawn_guarded(&self.runtime.spawner, task));
            tracing::debug!(feed = %entry.key, kind = ?entry.request.kind, "resource store feed started");
        }
        sub.attach_part(part, &st);
        drop(st);
        drop(entries);
        drop(evicted);
        entry
    }

    /// A new entry, after asking the budget (evicting idle feeds while it refuses).
    fn create(
        &self,
        key: &FeedKey,
        entries: &mut HashMap<FeedKey, Arc<FeedEntry>>,
        evicted: &mut Vec<(Option<TaskGuard>, Option<TaskGuard>)>,
    ) -> Arc<FeedEntry> {
        let plan = self.plan(&key.gvk);
        let mut request = FeedRequest {
            key: key.clone(),
            kind: plan.kind,
            priority: plan.priority,
        };
        loop {
            let running = entries.values().filter(|e| e.admitted).count();
            match self.options.budget.admit(&request, running) {
                Admission::Granted => break,
                Admission::Degraded(kind) => {
                    request.kind = kind;
                    break;
                }
                Admission::Refused(reason) => {
                    if let Some(guards) = self.evict_oldest_idle(entries) {
                        evicted.push(guards);
                        continue;
                    }
                    tracing::info!(feed = %key, "resource store feed refused by the watch budget");
                    let state = FeedState::Failed {
                        kind: ErrorKind::BudgetExceeded,
                        message: reason,
                    };
                    return Arc::new(FeedEntry::new(key.clone(), request, false, state));
                }
            }
        }
        Arc::new(FeedEntry::new(
            key.clone(),
            request,
            true,
            FeedState::Warming,
        ))
    }

    /// Removes the idle entry that has been idle longest; returns its guards to drop.
    fn evict_oldest_idle(
        &self,
        entries: &mut HashMap<FeedKey, Arc<FeedEntry>>,
    ) -> Option<(Option<TaskGuard>, Option<TaskGuard>)> {
        let oldest = entries
            .values()
            .filter_map(|e| {
                let st = e.state.lock();
                (st.subscribers.is_empty() && e.admitted)
                    .then(|| (st.idle_since.unwrap_or(Timestamp::MIN), e.key.clone()))
            })
            .min()?;
        let entry = entries.remove(&oldest.1)?;
        tracing::debug!(feed = %entry.key, "resource store evicted an idle feed for the budget");
        Some(self.retire(&entry))
    }

    /// Releases the budget slot and takes the entry's task guards (dropping them aborts).
    fn retire(&self, entry: &FeedEntry) -> (Option<TaskGuard>, Option<TaskGuard>) {
        let mut st = entry.state.lock();
        st.running = false;
        if entry.admitted {
            self.options.budget.released(&entry.request);
        }
        (st.driver.take(), st.grace.take())
    }

    /// Removes subscriber `id` from `entry`; the last one starts the grace timer.
    pub fn detach(self: &Arc<Self>, entry: &Arc<FeedEntry>, id: SubId) {
        let generation = {
            let mut st = entry.state.lock();
            st.subscribers.retain(|(sub, _)| *sub != id);
            if !st.subscribers.is_empty() {
                return;
            }
            st.generation += 1;
            let now = self.runtime.clock.now();
            st.idle_since = Some(now);
            let grace = self.options.config.idle_grace;
            if entry.admitted && !grace.is_zero() {
                let store = Arc::downgrade(self);
                let clock = self.runtime.clock.clone();
                let (key, generation) = (entry.key.clone(), st.generation);
                // Measured from now, not from when the task first runs.
                let deadline = now.checked_add(grace).unwrap_or(Timestamp::MAX);
                st.grace = Some(spawn_guarded(&self.runtime.spawner, async move {
                    clock.sleep_until(deadline).await;
                    reap(&store, &key, generation);
                }));
                return;
            }
            st.generation
        };
        self.reap(&entry.key, generation);
    }

    /// Tears down `key`'s entry if it is still idle since `generation`.
    fn reap(&self, key: &FeedKey, generation: u64) {
        let mut entries = self.entries.lock();
        let Some(entry) = entries.get(key).cloned() else {
            return;
        };
        {
            let st = entry.state.lock();
            if !st.subscribers.is_empty() || st.generation != generation {
                return;
            }
        }
        entries.remove(key);
        let guards = self.retire(&entry);
        drop(entries);
        tracing::debug!(feed = %key, "resource store feed stopped");
        drop(guards);
    }
}

fn reap(store: &Weak<StoreInner>, key: &FeedKey, generation: u64) {
    if let Some(store) = store.upgrade() {
        store.reap(key, generation);
    }
}
