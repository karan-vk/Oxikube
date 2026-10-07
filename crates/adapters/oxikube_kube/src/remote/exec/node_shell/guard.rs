//! Deleting the shell pod, whichever way the session ends, and stamping it alive until then.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use super::super::pods::Pods;
use super::heartbeat::Heartbeat;
use super::live::LiveShells;

/// The longest a cleanup delete may take. The pod also has `activeDeadlineSeconds` and the
/// leftover sweep, so giving up is safe.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Owns the obligation to delete one pod, and keeps the pod stamped alive meanwhile.
/// [`run`](Self::run) deletes it and waits for the answer; dropping the guard before that
/// answer came (the session was dropped, or aborted, even mid-delete) spawns the delete on the
/// runtime the guard was made in. Either way the stamps stop and the pod leaves the
/// [`LiveShells`] once its delete was answered.
pub(super) struct PodCleanup {
    pods: Arc<dyn Pods>,
    live: Arc<LiveShells>,
    id: u64,
    namespace: String,
    name: String,
    runtime: Handle,
    armed: bool,
    // Dropped (stopping the stamps) with the guard.
    _heartbeat: Heartbeat,
}

impl PodCleanup {
    /// A guard for `namespace/name`, listed in `live` and stamping it alive. Must be created
    /// inside a Tokio runtime.
    pub(super) fn new(
        pods: Arc<dyn Pods>,
        live: &Arc<LiveShells>,
        namespace: &str,
        name: &str,
    ) -> Self {
        let runtime = Handle::current();
        Self {
            id: live.add(namespace, name),
            live: live.clone(),
            _heartbeat: Heartbeat::start(
                &runtime,
                pods.clone(),
                namespace.to_owned(),
                name.to_owned(),
            ),
            pods,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            runtime,
            armed: true,
        }
    }

    /// Deletes the pod now. The guard stays armed until the delete has finished, so a `run`
    /// future that is dropped half way is covered by the drop fallback like any other.
    pub(super) async fn run(mut self) {
        delete(self.pods.as_ref(), &self.namespace, &self.name).await;
        self.live.remove(self.id);
        self.armed = false;
    }
}

impl Drop for PodCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pods = self.pods.clone();
        let live = self.live.clone();
        let id = self.id;
        let (namespace, name) = (
            std::mem::take(&mut self.namespace),
            std::mem::take(&mut self.name),
        );
        // Detached on purpose: this is the cleanup that must outlive its owner.
        drop(self.runtime.spawn(async move {
            delete(pods.as_ref(), &namespace, &name).await;
            live.remove(id);
        }));
    }
}

/// Deletes `namespace/name`, giving up after [`CLEANUP_TIMEOUT`]. `true` when the cluster
/// answered that the pod is gone.
pub(super) async fn delete(pods: &dyn Pods, namespace: &str, name: &str) -> bool {
    match tokio::time::timeout(CLEANUP_TIMEOUT, pods.delete(namespace, name)).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            tracing::warn!(
                namespace, pod = name, error = %err,
                "could not delete the node shell pod; the leftover sweep or its deadline will"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                namespace,
                pod = name,
                "deleting the node shell pod timed out; the leftover sweep or its deadline will"
            );
            false
        }
    }
}
