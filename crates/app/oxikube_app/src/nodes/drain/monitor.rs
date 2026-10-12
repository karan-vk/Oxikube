//! [`DrainMonitor`]: who follows which running drain.
//!
//! The `node::Drain` handler runs the drain inside its dispatch (the guard audits it as one
//! command); the dialog that started it, or any other follower, needs every step as it happens.
//! The handler publishes each step here under (cluster, node), and followers subscribe before
//! they dispatch. One drain per node at a time: a second one is refused while the first runs.

use std::collections::HashMap;
use std::sync::Arc;

use futures::channel::mpsc;
use oxikube_domain::drain::DrainProgress;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::Mutex;

type Key = (ClusterId, String);

#[derive(Default)]
struct Entry {
    /// Whether a drain of this node is running.
    running: bool,
    followers: Vec<mpsc::UnboundedSender<DrainProgress>>,
}

/// The running drains and their followers. Cheap to clone; clones share the table.
#[derive(Clone, Default)]
pub struct DrainMonitor {
    entries: Arc<Mutex<HashMap<Key, Entry>>>,
}

impl std::fmt::Debug for DrainMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrainMonitor")
            .field("nodes", &self.entries.lock().len())
            .finish()
    }
}

impl DrainMonitor {
    /// An empty monitor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every step of the next (or the running) drain of `node` on `cluster`, from now on. The
    /// receiver ends when that drain ends (its publisher is dropped) and it is dropped by the
    /// follower to stop following; the steps are not coalesced here, the view batches them.
    pub fn subscribe(
        &self,
        cluster: &ClusterId,
        node: &str,
    ) -> mpsc::UnboundedReceiver<DrainProgress> {
        let (tx, rx) = mpsc::unbounded();
        let mut entries = self.entries.lock();
        let followers = &mut entries
            .entry((cluster.clone(), node.to_owned()))
            .or_default()
            .followers;
        // Followers whose drain was refused before it started never got a step: forget the
        // ones that stopped listening.
        followers.retain(|follower| !follower.is_closed());
        followers.push(tx);
        rx
    }

    /// Whether a drain of `node` on `cluster` is running.
    pub fn is_running(&self, cluster: &ClusterId, node: &str) -> bool {
        self.entries
            .lock()
            .get(&(cluster.clone(), node.to_owned()))
            .is_some_and(|entry| entry.running)
    }

    /// Marks a drain of `node` as running and returns where its steps go.
    ///
    /// # Errors
    ///
    /// `Conflict` while another drain of the same node runs.
    pub(crate) fn start(&self, cluster: &ClusterId, node: &str) -> OxiResult<DrainPublisher> {
        let key = (cluster.clone(), node.to_owned());
        let mut entries = self.entries.lock();
        let entry = entries.entry(key.clone()).or_default();
        if entry.running {
            return Err(OxiError::conflict(format!(
                "node {node} is being drained already"
            )));
        }
        entry.running = true;
        Ok(DrainPublisher {
            monitor: self.clone(),
            key,
        })
    }
}

/// The publishing end of one running drain. Dropping it ends the drain for the followers (their
/// receivers end) and lets the node be drained again.
pub(crate) struct DrainPublisher {
    monitor: DrainMonitor,
    key: Key,
}

impl DrainPublisher {
    /// Hands `step` to every follower; followers that went away are forgotten.
    pub(crate) fn publish(&self, step: &DrainProgress) {
        let mut entries = self.monitor.entries.lock();
        if let Some(entry) = entries.get_mut(&self.key) {
            entry
                .followers
                .retain(|follower| follower.unbounded_send(step.clone()).is_ok());
        }
    }
}

impl Drop for DrainPublisher {
    fn drop(&mut self) {
        // The followers' receivers end with their senders; the entry goes.
        self.monitor.entries.lock().remove(&self.key);
    }
}
