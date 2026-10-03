//! [`ClientPool`]: one lazily built, shared kube [`Client`] per kubeconfig context.
//!
//! # Lifecycle
//!
//! The pool holds the current merged [`Kubeconfig`]. [`ClientPool::get`] slices the
//! requested context out of it ([`ContextDefinition`]), builds the client on first
//! use and hands out the same `Arc<Client>` afterwards. Concurrent `get`s for one
//! context share a per-entry async once-cell, so the client is built exactly once;
//! a failed build leaves the cell empty and the next `get` retries.
//!
//! Building blocks (certificate files, exec credential plugins), so it runs on
//! `tokio::task::spawn_blocking`. `get` must be called inside a Tokio runtime,
//! which in the app means through `oxikube_runtime::spawn_kube`.
//!
//! # Why `Arc<Client>`
//!
//! kube's `Client` is already a cheap clone (a handle on a shared tower buffer), but
//! a clone does not tell the pool whether anyone still uses it. Handing out
//! `Arc<Client>` gives pointer identity for "same client" checks and a strong
//! count, which the eviction policy uses to never drop a client someone holds.
//!
//! # Invalidation
//!
//! * [`ClientPool::invalidate`] drops one context's entry.
//! * [`ClientPool::replace_kubeconfig`] installs a new merged kubeconfig and drops
//!   only the entries whose context, cluster or user entry changed (credentials
//!   included) or disappeared. E03-S02's `SourcesChanged` drives this. Files the
//!   kubeconfig only points at (a `certificate-authority` path, a token file) are
//!   not compared; kube re-reads token files itself, and `invalidate` covers the
//!   rest.
//!
//! Dropping an entry never breaks a caller: whoever holds the old `Arc<Client>`
//! keeps a working client until they let go, and the next `get` builds a new one.
//!
//! # Eviction
//!
//! [`EvictionPolicy`] (in [`PoolConfig`]) bounds idle time and entry count. The
//! pool sweeps on every `get`; call [`ClientPool::evict_idle`] from a periodic task
//! to sweep while nothing calls `get`. In-use entries are never evicted: a held
//! client, a running build, or a context pinned with [`ClientPool::set_pinned`].
//! Idle time counts from the last sweep that saw the entry in use, so a client
//! released after long use gets the full `max_idle`; sweep at least every
//! `max_idle` to keep that accurate.
//!
//! # Secrets
//!
//! Entries hold credentials. Every `Debug` here prints context names and server
//! hosts only, and build errors never quote kube's messages that may contain
//! exec-plugin output.

mod build;
mod config;
mod entry;
mod eviction;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use kube::Client;
use kube::config::Kubeconfig;
use oxikube_domain::OxiError;
use oxikube_domain::ids::ContextName;
use parking_lot::Mutex;
use tokio::sync::OnceCell;

pub use build::{ClientFactory, KubeClientFactory, ProxyEnv, build_client, build_config};
pub use config::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_WRITE_TIMEOUT, PoolConfig, RetryMode};
pub use entry::ContextDefinition;
pub use eviction::{Clock, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_IDLE, EvictionPolicy, SystemClock};

use entry::PoolEntry;
use eviction::Candidate;

/// How many times `get` rebuilds when the entry is invalidated while its build runs.
const MAX_GET_ATTEMPTS: usize = 3;

/// One lazily built, shared kube client per kubeconfig context. See the module docs.
pub struct ClientPool {
    state: Mutex<State>,
    config: PoolConfig,
    factory: Arc<dyn ClientFactory>,
    clock: Arc<dyn Clock>,
}

/// Everything behind the lock. The lock is never held across an `.await`.
struct State {
    kubeconfig: Kubeconfig,
    entries: HashMap<ContextName, PoolEntry>,
    pinned: HashSet<ContextName>,
}

impl ClientPool {
    /// A pool over `kubeconfig` using the real kube factory (with the process's
    /// `HTTPS_PROXY` as proxy fallback) and the system clock.
    pub fn new(kubeconfig: Kubeconfig, config: PoolConfig) -> Self {
        Self::with_parts(
            kubeconfig,
            config,
            Arc::new(KubeClientFactory::from_process_env()),
            Arc::new(SystemClock),
        )
    }

    /// A pool with an explicit client factory and clock (tests, custom wiring).
    pub fn with_parts(
        kubeconfig: Kubeconfig,
        config: PoolConfig,
        factory: Arc<dyn ClientFactory>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                kubeconfig,
                entries: HashMap::new(),
                pinned: HashSet::new(),
            }),
            config,
            factory,
            clock,
        }
    }

    /// The settings applied to every client.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// The client for `context`, built on first use and shared afterwards.
    ///
    /// Errors: [`NotFound`](oxikube_domain::ErrorKind::NotFound) when the context is
    /// not in the kubeconfig; otherwise whatever the build reports (an invalid
    /// entry, failed credentials). Cancelling the future mid-build is safe: the
    /// next `get` builds again.
    pub async fn get(&self, context: &ContextName) -> Result<Arc<Client>, OxiError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (cell, definition) = self.checkout(context)?;
            let client = cell
                .get_or_try_init(|| self.build(definition))
                .await?
                .clone();
            // An `invalidate` or `replace_kubeconfig` that ran during the build
            // replaced or removed the entry; this client came from the old
            // definition, so build again from the current one.
            if attempt >= MAX_GET_ATTEMPTS || self.is_current(context, &cell) {
                return Ok(client);
            }
        }
    }

    /// Drops the entry for `context`. Returns whether there was one. Holders of
    /// the old client keep it; the next `get` rebuilds.
    pub fn invalidate(&self, context: &ContextName) -> bool {
        self.state.lock().entries.remove(context).is_some()
    }

    /// Installs a new merged kubeconfig and drops the entries whose definition
    /// changed or vanished. Unchanged entries keep their client. Returns the
    /// dropped contexts, sorted.
    pub fn replace_kubeconfig(&self, kubeconfig: Kubeconfig) -> Vec<ContextName> {
        let mut state = self.state.lock();
        let mut dropped: Vec<ContextName> = state
            .entries
            .iter()
            .filter(|(name, entry)| {
                ContextDefinition::from_kubeconfig(&kubeconfig, name)
                    .is_none_or(|new| !new.same_connection(&entry.definition))
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &dropped {
            state.entries.remove(name);
        }
        state.kubeconfig = kubeconfig;
        dropped.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        dropped
    }

    /// Pins or unpins `context`. A pinned context's client is never evicted (it is
    /// still dropped by invalidation). The pin outlives the entry, so it applies to
    /// clients built later too, and it survives `replace_kubeconfig` removing the
    /// context (a transient kubeconfig edit keeps the session's pin). The session
    /// layer decides what to pin and unpins when the session closes.
    pub fn set_pinned(&self, context: &ContextName, pinned: bool) {
        let mut state = self.state.lock();
        if pinned {
            state.pinned.insert(context.clone());
        } else {
            state.pinned.remove(context);
        }
    }

    /// Runs the eviction policy now. Returns the evicted contexts.
    pub fn evict_idle(&self) -> Vec<ContextName> {
        let mut state = self.state.lock();
        self.sweep(&mut state)
    }

    /// Number of cached entries (built or building).
    pub fn len(&self) -> usize {
        self.state.lock().entries.len()
    }

    /// Whether no entry is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `context` has a cached entry.
    pub fn contains(&self, context: &ContextName) -> bool {
        self.state.lock().entries.contains_key(context)
    }

    /// Finds or creates the entry, marks it used and returns what the build needs.
    /// The returned cell `Arc` marks the entry in use until the caller drops it.
    fn checkout(
        &self,
        context: &ContextName,
    ) -> Result<(Arc<OnceCell<Arc<Client>>>, Arc<ContextDefinition>), OxiError> {
        let now = self.clock.now();
        let mut state = self.state.lock();
        let found = if let Some(entry) = state.entries.get_mut(context) {
            entry.last_used = now;
            (entry.cell.clone(), entry.definition.clone())
        } else {
            let definition = ContextDefinition::from_kubeconfig(&state.kubeconfig, context)
                .ok_or_else(|| {
                    OxiError::not_found(format!("context `{context}` is not in the kubeconfig"))
                })?;
            let entry = PoolEntry::new(definition, now);
            let found = (entry.cell.clone(), entry.definition.clone());
            state.entries.insert(context.clone(), entry);
            found
        };
        // The cell clone above keeps this entry out of the sweep.
        self.sweep(&mut state);
        Ok(found)
    }

    fn is_current(&self, context: &ContextName, cell: &Arc<OnceCell<Arc<Client>>>) -> bool {
        self.state
            .lock()
            .entries
            .get(context)
            .is_some_and(|entry| Arc::ptr_eq(&entry.cell, cell))
    }

    async fn build(&self, definition: Arc<ContextDefinition>) -> Result<Arc<Client>, OxiError> {
        let factory = self.factory.clone();
        let config = self.config.clone();
        let context = definition.context().clone();
        tokio::task::spawn_blocking(move || factory.build(&definition, &config))
            .await
            .map_err(|join| {
                let what = if join.is_panic() {
                    "panicked"
                } else {
                    "was cancelled"
                };
                OxiError::internal(format!("building the client for `{context}` {what}"))
            })?
            .map(Arc::new)
    }

    fn sweep(&self, state: &mut State) -> Vec<ContextName> {
        let now = self.clock.now();
        // Idle time starts when an entry stops being in use, not at its last `get`:
        // a client held for an hour and then dropped must get the full `max_idle`.
        // Refreshing in-use entries here makes the idle clock start at the last
        // sweep that saw it in use, so the error is bounded by the sweep cadence.
        let State {
            entries, pinned, ..
        } = state;
        for (name, entry) in entries.iter_mut() {
            if pinned.contains(name) || entry.is_referenced() {
                entry.last_used = now;
            }
        }
        let candidates: Vec<Candidate<'_>> = entries
            .iter()
            .map(|(name, entry)| Candidate {
                context: name,
                last_used: entry.last_used,
                in_use: pinned.contains(name) || entry.is_referenced(),
            })
            .collect();
        let evicted = eviction::select(&self.config.eviction, now, &candidates);
        for name in &evicted {
            state.entries.remove(name);
        }
        evicted
    }
}

impl fmt::Debug for ClientPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        let mut entries: Vec<_> = state.entries.values().collect();
        entries.sort_unstable_by(|a, b| {
            a.definition
                .context()
                .as_str()
                .cmp(b.definition.context().as_str())
        });
        f.debug_struct("ClientPool")
            .field("entries", &entries)
            .field("pinned", &state.pinned.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
