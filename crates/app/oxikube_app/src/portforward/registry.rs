//! The manager's shared state: [`Inner`] (the entries, the config, the change signal) and the
//! one door through which the user's calls and the supervisors change an entry
//! ([`Inner::transition`]: the state machine step, the phase-change notification, the change
//! signal).

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::PortForwardChange;
use oxikube_domain::portforward::ForwardSpec;
use oxikube_ports::{ClockPort, ForwardStats, ForwardStatsSnapshot, NotificationSourcePort};
use parking_lot::Mutex;
use tokio::sync::watch;

use super::backend::ForwardBackend;
use super::model::{ForwardConfig, ForwardId, ForwardInfo, label};
use super::state::{ForwardEvent, ForwardState, step};
use crate::store::{Spawner, TaskGuard};

pub(super) struct Inner {
    pub(super) backend: Arc<dyn ForwardBackend>,
    pub(super) spawner: Arc<dyn Spawner>,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) notify: Option<Arc<dyn NotificationSourcePort>>,
    pub(super) config: Mutex<ForwardConfig>,
    /// Per-cluster overrides of `config` (`clusters.<id>.portforward`).
    pub(super) clusters: Mutex<HashMap<ClusterId, ForwardConfig>>,
    pub(super) entries: Mutex<BTreeMap<ForwardId, Entry>>,
    pub(super) next_id: AtomicU64,
    pub(super) changes: watch::Sender<u64>,
    /// Saves a forward as a favourite (set by [`PortForwardFavourites`]); `portforward::Start`
    /// with `favourite` calls it once the forward is up.
    ///
    /// [`PortForwardFavourites`]: super::PortForwardFavourites
    pub(super) favourite_saver: Mutex<Option<FavouriteSaver>>,
}

/// Saves a spec as an auto-start favourite: the favourites' `add`, held weakly by the manager.
pub(super) type FavouriteSaver =
    Arc<dyn Fn(ForwardSpec) -> BoxFuture<'static, OxiResult<()>> + Send + Sync>;

pub(super) struct Entry {
    pub(super) spec: ForwardSpec,
    pub(super) state: ForwardState,
    pub(super) local_addr: Option<SocketAddr>,
    pub(super) pod: Option<String>,
    pub(super) last_error: Option<String>,
    pub(super) stats: Option<Arc<ForwardStats>>,
    /// Traffic of the handles before the current one.
    pub(super) carried: ForwardStatsSnapshot,
    /// Bumped on every (re)start, so a stopped supervisor's late update is ignored.
    pub(super) generation: u64,
    pub(super) task: Option<TaskGuard>,
    /// Asks the running supervisor to shut its handle down gracefully (the user's stop).
    pub(super) stop: Option<futures::channel::oneshot::Sender<()>>,
    /// When the forward was started or last restarted.
    pub(super) started_at: jiff::Timestamp,
}

impl Entry {
    pub(super) fn info(&self, id: ForwardId) -> ForwardInfo {
        let live = self
            .stats
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_default();
        ForwardInfo {
            id,
            spec: self.spec.clone(),
            state: self.state.clone(),
            local_addr: self.local_addr,
            pod: self.pod.clone(),
            last_error: self.last_error.clone(),
            stats: ForwardStatsSnapshot {
                bytes_in: self.carried.bytes_in + live.bytes_in,
                bytes_out: self.carried.bytes_out + live.bytes_out,
                active_connections: live.active_connections,
                total_connections: self.carried.total_connections + live.total_connections,
            },
            started_at: self.started_at,
        }
    }
}

impl Inner {
    /// Signals a change to the subscribers.
    pub(super) fn bump(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Applies `event` to `id`'s state (when `generation` is current, or `None` for the user's
    /// own calls), then `also` to the entry; notifies a phase change. Returns whether `id` is
    /// listed (and current).
    pub(super) fn transition(
        &self,
        id: ForwardId,
        generation: Option<u64>,
        event: ForwardEvent,
        also: impl FnOnce(&mut EntryMut<'_>),
    ) -> bool {
        let change = {
            let mut entries = self.entries.lock();
            let Some(entry) = entries.get_mut(&id) else {
                return false;
            };
            if generation.is_some_and(|g| g != entry.generation) {
                return false;
            }
            let from = entry.state.clone();
            let config = *self.config.lock();
            let unit = jitter_unit(id, entry.generation, &from);
            entry.state = step(&from, event, &config.backoff, unit);
            also(&mut EntryMut(entry));
            if let ForwardState::Active { pod, local_addr } = &entry.state {
                entry.pod = Some(pod.clone());
                entry.local_addr = Some(*local_addr);
            }
            (from.phase() != entry.state.phase()).then(|| PortForwardChange {
                cluster: entry.spec.target.cluster.clone(),
                forward_id: id.to_string(),
                label: label(&entry.spec, entry.local_addr),
                target: Some(entry.spec.target.clone()),
                from: from.phase(),
                to: entry.state.phase(),
                reason: entry.state.reason().map(str::to_owned),
            })
        };
        if let (Some(change), Some(notify)) = (change, &self.notify) {
            notify.port_forward_changed(change);
        }
        self.bump();
        true
    }

    /// The state of `id` when `generation` is current.
    pub(super) fn state(&self, id: ForwardId, generation: u64) -> Option<ForwardState> {
        let entries = self.entries.lock();
        let entry = entries.get(&id)?;
        (entry.generation == generation).then(|| entry.state.clone())
    }

    /// Edits `id`'s entry when `generation` is current, without a state change.
    pub(super) fn edit(
        &self,
        id: ForwardId,
        generation: u64,
        edit: impl FnOnce(&mut EntryMut<'_>),
    ) {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get_mut(&id).filter(|e| e.generation == generation) {
            edit(&mut EntryMut(entry));
            drop(entries);
            self.bump();
        }
    }
}

/// What a supervisor may change in its entry besides the state.
pub(super) struct EntryMut<'a>(pub(super) &'a mut Entry);

impl EntryMut<'_> {
    /// The counters of a new handle; the previous handle's traffic is carried over.
    pub(super) fn set_stats(&mut self, stats: Option<Arc<ForwardStats>>) {
        if let Some(old) = self.0.stats.take() {
            let s = old.snapshot();
            self.0.carried.bytes_in += s.bytes_in;
            self.0.carried.bytes_out += s.bytes_out;
            self.0.carried.total_connections += s.total_connections;
        }
        self.0.stats = stats;
    }

    /// The last error one connection hit while the forward kept running.
    pub(super) fn set_last_error(&mut self, error: Option<String>) {
        self.0.last_error = error;
    }
}

/// A deterministic point in `[0, 1)` for the backoff jitter, different per forward and attempt.
fn jitter_unit(id: ForwardId, generation: u64, state: &ForwardState) -> f64 {
    let attempt = match state {
        ForwardState::Reconnecting { attempt, .. } => u64::from(*attempt),
        _ => 0,
    };
    let mut x = id.0 ^ generation.rotate_left(17) ^ attempt.rotate_left(33) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}
