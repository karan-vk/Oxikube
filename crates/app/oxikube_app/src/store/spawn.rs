//! [`Spawner`]: how the store runs its feed tasks without naming a runtime, and the
//! abort-on-drop [`TaskGuard`] it keeps for each of them.
//!
//! `oxikube_app` is plain async Rust: it never names tokio's spawn or GPUI. The binary hands the
//! store a [`Spawner`] that puts a future on the Tokio bridge (`oxikube_runtime`'s handle, the
//! same runtime `spawn_kube` uses); tests hand it a queue they poll themselves. Every task the
//! store spawns is wrapped in [`futures::future::Abortable`], and the store keeps the matching
//! [`TaskGuard`]: dropping the guard aborts the task, which drops the feed it was reading
//! (non-negotiable 7). No task ever drops its own guard.

use std::sync::Arc;

use futures::future::{AbortHandle, Abortable, BoxFuture, FutureExt};

/// Runs a detached future on some executor.
///
/// Implemented for any `Fn(BoxFuture<'static, ()>)`, so the binary can pass a closure over a
/// Tokio handle:
///
/// ```ignore
/// let handle = oxikube_runtime::handle(cx).expect("tokio runtime");
/// let spawner: Arc<dyn Spawner> = Arc::new(move |task| {
///     handle.spawn(task);
/// });
/// ```
///
/// The store never relies on the spawned task's handle: it cancels through the `TaskGuard`
/// it keeps, so an implementation may detach the task.
pub trait Spawner: Send + Sync {
    /// Starts `task` and returns at once; `task` must not be polled on the caller's stack.
    fn spawn(&self, task: BoxFuture<'static, ()>);
}

impl<F> Spawner for F
where
    F: Fn(BoxFuture<'static, ()>) + Send + Sync,
{
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        self(task);
    }
}

/// Aborts its task when dropped (abort-on-drop, non-negotiable 7).
#[derive(Debug)]
pub(crate) struct TaskGuard(AbortHandle);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawns `task` on `spawner`, abortable through the returned guard.
pub(crate) fn spawn_guarded(
    spawner: &Arc<dyn Spawner>,
    task: impl Future<Output = ()> + Send + 'static,
) -> TaskGuard {
    let (handle, registration) = AbortHandle::new_pair();
    spawner.spawn(Abortable::new(task, registration).map(|_| ()).boxed());
    TaskGuard(handle)
}
