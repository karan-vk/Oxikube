//! [`spawn_kube`]: run a future on the bridge runtime and get its result back as a GPUI [`Task`].
//!
//! The returned [`KubeTask`] owns the work. Dropping it aborts the tokio task (its future is
//! dropped at its next `.await`, closing watches and HTTP streams), so a view that stores the task
//! in a field cancels its Kubernetes work simply by being dropped. `.detach()` lets the work run to
//! completion with nobody listening; only do that for fire-and-forget work that must finish.
//!
//! Awaiting the task from a foreground `cx.spawn` resumes on the main thread with the result, ready
//! to `update` an entity:
//!
//! ```ignore
//! self.load = cx.spawn(async move |this, cx| {
//!     let pods = spawn_kube(cx, async move { port.list(&target).await }).await;
//!     this.update(cx, |this, cx| this.apply(pods, cx)).ok();
//! });
//! ```

use crate::gpui_tokio::{Backend, GlobalTokio};
use futures::FutureExt as _;
use gpui::{App, AppContext, Task};
use oxikube_domain::OxiError;
use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, Location};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task::{AbortHandle, JoinError};
use tracing::Instrument as _;

/// How many [`spawn_kube`] futures of this app are alive: started and neither finished nor aborted
/// (a dropped [`KubeTask`] aborts its future, which ends it). The leak tests read it before and
/// after a batch of open/close cycles and assert it is back where it started; it is also a cheap
/// health number for `--perf`. `0` before any `init*` call. Counted per app, so tests running side
/// by side do not see each other's tasks.
pub fn live_tasks(cx: &App) -> usize {
    cx.try_global::<GlobalTokio>()
        .map_or(0, |global| global.live.load(Ordering::Acquire))
}

/// Counts one live [`spawn_kube`] future for as long as it exists (finished or dropped).
struct LiveGuard(Arc<AtomicUsize>);

impl LiveGuard {
    fn new(live: &Arc<AtomicUsize>) -> Self {
        live.fetch_add(1, Ordering::AcqRel);
        Self(live.clone())
    }
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The GPUI task [`spawn_kube`] returns: the future's output, or why it never produced one.
pub type KubeTask<R> = Task<Result<R, KubeTaskError>>;

/// Why a [`spawn_kube`] future produced no output.
///
/// Dropping the [`KubeTask`] itself never yields this: a dropped task has nobody to report to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KubeTaskError {
    /// The future panicked. The message is the panic payload, passed through
    /// [`oxikube_domain::redact::redact`].
    #[error("kube task panicked: {0}")]
    Panicked(String),
    /// The tokio runtime shut down (the app is quitting) before the future finished.
    #[error("kube task cancelled: the tokio runtime shut down")]
    Cancelled,
}

impl From<KubeTaskError> for OxiError {
    /// Both cases are bugs or shutdown, never user-facing conditions: `Internal`, not retryable
    /// (ARCHITECTURE.md error table).
    fn from(err: KubeTaskError) -> Self {
        OxiError::internal(err.to_string())
    }
}

impl From<JoinError> for KubeTaskError {
    fn from(err: JoinError) -> Self {
        match err.try_into_panic() {
            Ok(payload) => KubeTaskError::Panicked(panic_message(payload.as_ref())),
            // The only other JoinError is cancellation: an abort (we only abort when the KubeTask
            // is dropped, and then nobody awaits it) or a runtime shutdown.
            Err(_) => KubeTaskError::Cancelled,
        }
    }
}

/// Runs `fut` on the bridge runtime and returns a GPUI task resolving to its output.
///
/// - Tokio mode ([`crate::init`], [`crate::init_from_handle`]): `fut` runs on a tokio worker;
///   dropping the returned task aborts the tokio task.
/// - Deterministic mode ([`crate::init_deterministic`]): `fut` is polled on GPUI's background
///   executor; dropping the task drops the future. Same contract, no OS threads.
///
/// Panics in `fut` come back as [`KubeTaskError::Panicked`] in both modes. Each task runs inside a
/// `spawn_kube` tracing span carrying the caller's source location, so `--perf` and tracing
/// subscribers can attribute the work.
///
/// # Panics
///
/// If no `init*` function ran on this `App` (the same contract as GPUI's `cx.global()`): the
/// binary initialises the runtime first in its init order (E05-S09), tests call
/// [`crate::init_deterministic`].
#[track_caller]
pub fn spawn_kube<C, Fut, R>(cx: &C, fut: Fut) -> KubeTask<R>
where
    C: AppContext,
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let caller = Location::caller();
    let span = tracing::debug_span!("spawn_kube", caller = %caller);
    cx.read_global(|global: &GlobalTokio, app| {
        // Dropped with the future: when it finishes, panics or is aborted.
        let live = LiveGuard::new(&global.live);
        let fut = async move {
            let _live = live;
            fut.await
        };
        match &global.backend {
            Backend::Tokio { handle, .. } => {
                let join = handle.spawn(fut.instrument(span));
                let abort = AbortOnDrop(join.abort_handle());
                app.background_executor().spawn(async move {
                    let result = join.await;
                    // Disarm only once the tokio task finished; until then, dropping this future (the
                    // GPUI task was dropped) drops `abort`, which aborts the tokio task.
                    drop(abort);
                    result.map_err(KubeTaskError::from)
                })
            }
            Backend::Deterministic => app.background_executor().spawn(async move {
                AssertUnwindSafe(fut.instrument(span))
                    .catch_unwind()
                    .await
                    .map_err(|payload| KubeTaskError::Panicked(panic_message(payload.as_ref())))
            }),
        }
    })
}

/// Aborts the tokio task when dropped.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        // A no-op when the task already finished.
        self.0.abort();
    }
}

/// The text of a panic payload (`&str` or `String`; anything else is opaque), redacted.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    let raw = if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "non-string panic payload"
    };
    oxikube_domain::redact::redact(raw).into_owned()
}
