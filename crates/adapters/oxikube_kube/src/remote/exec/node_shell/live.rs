//! The node-shell pods an adapter has open right now, so the app's quit can delete them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::join_all;
use parking_lot::Mutex;

use super::super::pods::Pods;
use super::guard::delete;

/// The shells of one [`KubeExec`](super::super::KubeExec) (and its clones) that have not
/// finished cleaning up: each [`PodCleanup`](super::guard::PodCleanup) is listed from the
/// moment its pod exists until its delete has been answered.
#[derive(Debug, Default)]
pub(in crate::remote::exec) struct LiveShells {
    next: AtomicU64,
    pods: Mutex<HashMap<u64, (String, String)>>,
}

impl LiveShells {
    /// Lists `namespace/name`; the id [`remove`](Self::remove)s it.
    pub(super) fn add(&self, namespace: &str, name: &str) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.pods
            .lock()
            .insert(id, (namespace.to_owned(), name.to_owned()));
        id
    }

    /// Forgets a pod that is gone.
    pub(super) fn remove(&self, id: u64) {
        self.pods.lock().remove(&id);
    }

    /// How many shells are open.
    #[cfg(test)]
    pub(in crate::remote::exec) fn len(&self) -> usize {
        self.pods.lock().len()
    }

    /// Deletes every listed pod at once and waits for the answers (each bounded by the cleanup
    /// timeout). Returns how many deletes went through. The pods stay listed until their own
    /// cleanup finishes; deleting a pod twice is harmless.
    pub(in crate::remote::exec) async fn release(&self, pods: &dyn Pods) -> usize {
        let open: Vec<(String, String)> = self.pods.lock().values().cloned().collect();
        let outcomes = join_all(
            open.iter()
                .map(|(namespace, name)| delete(pods, namespace, name)),
        )
        .await;
        outcomes.into_iter().filter(|deleted| *deleted).count()
    }
}
