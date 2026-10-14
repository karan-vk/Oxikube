//! What the feed task and the readers do to the service's state: apply a batch (a relist is
//! built outside the lock and swapped in), record the feed's state and monitor, read a cluster's
//! buffer, and apply a new config.

use std::sync::Arc;

use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{DeltaBatch, EventsMonitor, EventsStatus};

use super::buffer::EventBuffer;
use super::changes::Change;
use super::cluster::{Inner, State, wake};
use super::config::EventsConfig;
use super::filter::EventFilter;
use super::state::EventsState;

impl Inner {
    /// Applies one feed batch of feed `generation`. A relist is built into a new buffer outside
    /// the lock (it is the one large piece of work) and swapped in.
    pub(crate) fn apply_batch(
        &self,
        cluster: &ClusterId,
        generation: u64,
        batch: DeltaBatch<Arc<Event>>,
    ) {
        let mut deltas = batch.deltas;
        // Everything before the last relist is replaced by it.
        let rebuilt = deltas.iter().rposition(|d| d.is_restart()).map(|at| {
            let mut buffer = EventBuffer::new(self.state.lock().config.buffer_for(cluster));
            for delta in deltas.drain(at..) {
                buffer.apply(delta);
            }
            buffer
        });
        let mut changes: Vec<Change> = Vec::new();
        let mut wakers = Vec::new();
        let old = {
            let mut state = self.state.lock();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            if entry.generation != generation || entry.feed.is_none() {
                return;
            }
            let old = rebuilt.map(|buffer| std::mem::replace(&mut entry.buffer, buffer));
            if old.is_some() {
                wakers.extend(entry.subscribers.values().filter_map(|s| s.push_reset()));
            } else {
                for delta in deltas {
                    entry.buffer.apply_into(delta, &mut changes);
                }
                wakers.extend(entry.subscribers.values().filter_map(|s| s.push(&changes)));
            }
            entry.set_state(EventsState::Ready, &mut wakers);
            old
        };
        drop(old);
        wake(wakers);
    }

    /// Sets the state of feed `generation`.
    pub(crate) fn set_state(&self, cluster: &ClusterId, generation: u64, feed: EventsState) {
        let mut wakers = Vec::new();
        if let Some(entry) = self.state.lock().clusters.get_mut(cluster)
            && entry.generation == generation
        {
            entry.set_state(feed, &mut wakers);
        }
        wake(wakers);
    }

    /// Keeps the monitor of feed `generation`. `false` when that feed is no longer current.
    pub(crate) fn set_monitor(
        &self,
        cluster: &ClusterId,
        generation: u64,
        monitor: Arc<dyn EventsMonitor>,
    ) -> bool {
        let mut state = self.state.lock();
        match state.clusters.get_mut(cluster) {
            Some(entry) if entry.generation == generation => {
                entry.monitor = Some(monitor);
                true
            }
            _ => false,
        }
    }

    /// Runs `read` on `cluster`'s buffer, if the cluster has one.
    pub(crate) fn read<R>(
        &self,
        cluster: &ClusterId,
        read: impl FnOnce(&EventBuffer) -> R,
    ) -> Option<R> {
        self.state
            .lock()
            .clusters
            .get(cluster)
            .map(|e| read(&e.buffer))
    }

    pub(crate) fn latest(
        &self,
        cluster: &ClusterId,
        filter: &EventFilter,
        n: usize,
    ) -> Vec<Arc<Event>> {
        self.read(cluster, |b| b.query(filter).take(n).cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn state_of(&self, cluster: &ClusterId) -> EventsState {
        self.state
            .lock()
            .clusters
            .get(cluster)
            .map(|e| e.state.clone())
            .unwrap_or_default()
    }

    pub(crate) fn status_of(&self, cluster: &ClusterId) -> Option<EventsStatus> {
        let monitor = self.state.lock().clusters.get(cluster)?.monitor.clone()?;
        Some(monitor.status())
    }

    /// Replaces the config: a cluster whose bound changed is trimmed at once (shrinking evicts
    /// the oldest) and its running feed restarts with the new bound, so the adapter's ring
    /// agrees (growing keeps every event and relists to fill the room).
    pub(crate) fn set_config(self: &Arc<Self>, config: EventsConfig) {
        let mut wakers = Vec::new();
        let restart: Vec<ClusterId> = {
            let mut state = self.state.lock();
            state.config = config;
            let State {
                config, clusters, ..
            } = &mut *state;
            let mut restart = Vec::new();
            for (cluster, entry) in clusters.iter_mut() {
                let capacity = config.buffer_for(cluster);
                if entry.buffer.capacity() == capacity {
                    continue;
                }
                let mut changes = Vec::new();
                entry.buffer.set_capacity_into(capacity, &mut changes);
                wakers.extend(entry.subscribers.values().filter_map(|s| s.push(&changes)));
                if entry.feed.is_some() {
                    restart.push(cluster.clone());
                }
            }
            restart
        };
        wake(wakers);
        for cluster in restart {
            self.restart(&cluster);
        }
    }
}
