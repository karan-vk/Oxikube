//! Deleting the shell pod, whichever way the session ends.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use super::super::pods::Pods;

/// The longest a cleanup delete may take. The pod also has `activeDeadlineSeconds` and the
/// leftover sweep, so giving up is safe.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Owns the obligation to delete one pod. [`run`](Self::run) deletes it and waits for the
/// answer; dropping the guard without running it (the session was dropped, or aborted) spawns
/// the delete on the runtime the guard was made in.
pub(super) struct PodCleanup {
    pods: Arc<dyn Pods>,
    namespace: String,
    name: String,
    runtime: Handle,
    armed: bool,
}

impl PodCleanup {
    /// A guard for `namespace/name`. Must be created inside a Tokio runtime.
    pub(super) fn new(pods: Arc<dyn Pods>, namespace: &str, name: &str) -> Self {
        Self {
            pods,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            runtime: Handle::current(),
            armed: true,
        }
    }

    /// Deletes the pod now.
    pub(super) async fn run(mut self) {
        self.armed = false;
        delete(self.pods.as_ref(), &self.namespace, &self.name).await;
    }
}

impl Drop for PodCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pods = self.pods.clone();
        let (namespace, name) = (
            std::mem::take(&mut self.namespace),
            std::mem::take(&mut self.name),
        );
        // Detached on purpose: this is the cleanup that must outlive its owner.
        drop(self.runtime.spawn(async move {
            delete(pods.as_ref(), &namespace, &name).await;
        }));
    }
}

async fn delete(pods: &dyn Pods, namespace: &str, name: &str) {
    match tokio::time::timeout(CLEANUP_TIMEOUT, pods.delete(namespace, name)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => tracing::warn!(
            namespace, pod = name, error = %err,
            "could not delete the node shell pod; the leftover sweep or its deadline will"
        ),
        Err(_) => tracing::warn!(
            namespace,
            pod = name,
            "deleting the node shell pod timed out; the leftover sweep or its deadline will"
        ),
    }
}
