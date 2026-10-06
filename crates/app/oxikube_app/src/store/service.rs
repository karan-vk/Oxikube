//! [`ResourceStore`]: one cluster session's cache of feeds, keyed by (gvk, scope), with
//! ref-counted subscriptions, grace-period teardown and the budget hook.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_ports::{ApiWarning, WarningPort};
use parking_lot::Mutex;

use super::config::{FeedInfo, StoreOptions, StoreRuntime};
use super::delta::FeedState;
use super::driver::{Backoff, drive};
use super::entry::{EntryState, FeedEntry, SubId};
use super::feed::StorePorts;
use super::mailbox::SubShared;
use super::object::{FeedKey, FeedScope};
use super::policy::FeedPlan;
use super::query::StoreQuery;
use super::selector::LabelSelector;
use super::spawn::{TaskGuard, spawn_guarded};
use super::subscription::Subscription;
use super::warnings::{WarningLedger, distinct};
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
        Self::with_warnings(cluster, ports, runtime, options, None)
    }

    /// A store that also exposes the cluster's `Warning:` headers ([`warnings`](Self::warnings)).
    pub fn with_warnings(
        cluster: ClusterId,
        ports: StorePorts,
        runtime: StoreRuntime,
        options: StoreOptions,
        warnings: Option<Arc<dyn WarningPort>>,
    ) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                cluster,
                ports,
                runtime,
                options,
                warnings,
                ledger: Arc::new(WarningLedger::default()),
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
        Some(Self::with_warnings(
            session.id().clone(),
            ports,
            runtime,
            options,
            session.warnings(),
        ))
    }

    /// Whether `other` is a handle on this same store (not merely one for the same cluster).
    pub fn is_same_store(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(super) fn inner(&self) -> &Arc<StoreInner> {
        &self.inner
    }

    /// The cluster this store caches.
    pub fn cluster(&self) -> &ClusterId {
        &self.inner.cluster
    }

    /// Whether `other` is a handle on this same store (a reconnect builds a new one, so a view
    /// holding a store can tell it must re-subscribe).
    pub fn is_same(&self, other: &ResourceStore) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
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

    /// The API server's `Warning:` headers from now on, each distinct one once per store (so once
    /// per session, however many tables ask): a stream that yields the first of each
    /// (code, text) and swallows repeats. Empty when the session exposes no warnings.
    ///
    /// Subscribe before opening the feeds whose requests may warn: nothing is replayed.
    pub fn warnings(&self) -> BoxStream<'static, ApiWarning> {
        distinct(self.inner.warnings.as_ref(), self.inner.ledger.clone())
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
                    kind: st.request.kind,
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
    pub(super) options: StoreOptions,
    warnings: Option<Arc<dyn WarningPort>>,
    ledger: Arc<WarningLedger>,
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

    /// Runs `read` on the entry of `key` under its lock, if there is one (a cache read, never a
    /// feed start).
    pub fn with_entry<R>(&self, key: &FeedKey, read: impl FnOnce(&EntryState) -> R) -> Option<R> {
        let entry = self.entries.lock().get(key).cloned()?;
        let st = entry.state.lock();
        Some(read(&st))
    }

    /// Runs `task` on the store's spawner, abortable through the returned guard.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> TaskGuard {
        spawn_guarded(&self.runtime.spawner, task)
    }

    /// Adds subscriber `id` to the entry for (`gvk`, `part`), creating and starting it when
    /// needed (a new entry's driver first seeds it from `seed_from`'s objects in `part`), and
    /// registers the part with the subscriber. An entry the budget refused asks it again.
    pub fn attach(
        self: &Arc<Self>,
        gvk: &Gvk,
        part: &FeedScope,
        selector: Option<&LabelSelector>,
        id: SubId,
        sub: &Arc<SubShared>,
        seed_from: &[Arc<FeedEntry>],
    ) -> Arc<FeedEntry> {
        let key = FeedKey::new(gvk.clone(), part.clone()).with_selector(selector.cloned());
        let mut evicted = Vec::new();
        let mut entries = self.entries.lock();
        let created = !entries.contains_key(&key);
        if created {
            let entry = self.create(&key, &mut entries, &mut evicted);
            entries.insert(key.clone(), entry);
        }
        let entry = entries[&key].clone();
        if !created {
            self.readmit(&entry, &mut entries, &mut evicted);
        }
        let mut st = entry.state.lock();
        st.subscribers.push((id, sub.clone()));
        st.grace = None;
        st.idle_since = None;
        if st.admitted && !st.running {
            // A restart after a terminal error or a late admission: subscribers already
            // attached see it warm up.
            let seed_from = if created {
                seed_from.to_vec()
            } else {
                Vec::new()
            };
            self.start_driver(&entry, &mut st, seed_from);
        }
        sub.attach_part(part, &st);
        drop(st);
        drop(entries);
        drop(evicted);
        entry
    }

    /// Starts `entry`'s driver (replacing, and so aborting, any earlier one): the entry warms up,
    /// first seeded from `seed_from`. The caller holds the entry lock as `st`.
    fn start_driver(
        self: &Arc<Self>,
        entry: &Arc<FeedEntry>,
        st: &mut EntryState,
        seed_from: Vec<Arc<FeedEntry>>,
    ) {
        st.running = true;
        FeedEntry::publish(st, &entry.key, FeedState::Warming);
        let task = drive(
            Arc::downgrade(entry),
            self.ports.clone(),
            st.request.kind,
            entry.key.clone(),
            self.runtime.clock.clone(),
            Backoff::new(
                self.options.config.retry_initial,
                self.options.config.retry_max,
            ),
            seed_from,
        );
        st.driver = Some(spawn_guarded(&self.runtime.spawner, task));
        tracing::debug!(feed = %entry.key, kind = ?st.request.kind, "resource store feed started");
    }

    /// Restarts `entry` now unless it is `Ready`: a feed that stopped (forbidden, unauthorized,
    /// failed, refused by the budget) starts again, and one waiting out a retry backoff reopens
    /// at once with a fresh backoff. Its cached rows stay until the new list reconciles them.
    pub fn retry(self: &Arc<Self>, entry: &Arc<FeedEntry>) {
        let mut evicted = Vec::new();
        let mut entries = self.entries.lock();
        if !entries
            .get(&entry.key)
            .is_some_and(|e| Arc::ptr_eq(e, entry))
        {
            return;
        }
        self.readmit(entry, &mut entries, &mut evicted);
        let mut st = entry.state.lock();
        if st.admitted && !st.feed_state.is_ready() {
            self.start_driver(entry, &mut st, Vec::new());
        }
        drop(st);
        drop(entries);
        drop(evicted);
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
            if st.admitted && !grace.is_zero() {
                let store = Arc::downgrade(self);
                let clock = self.runtime.clock.clone();
                let (key, generation) = (entry.key.clone(), st.generation);
                // Measured from now, not from when the task first runs.
                let deadline = now.checked_add(grace).unwrap_or(Timestamp::MAX);
                st.grace = Some(spawn_guarded(&self.runtime.spawner, async move {
                    clock.sleep_until(deadline).await;
                    if let Some(store) = store.upgrade() {
                        store.reap(&key, generation);
                    }
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
