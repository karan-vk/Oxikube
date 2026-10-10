//! Keeps the registry current by watching `CustomResourceDefinition`s.
//!
//! A metadata-only watch (names and versions are all that matter; the CRD schemas can be
//! megabytes) turns every CRD change into a signal. Signals are debounced (a `helm install` of
//! an operator creates dozens of CRDs in a burst) and then trigger one
//! [`KubeDiscovery::refresh`], which publishes the registry diff. When nothing changes the task
//! is parked on the watch connection: no timers, no polling.
//!
//! A user who may not list or watch CRDs (common for namespace-scoped users) gets a
//! [`CrdWatchStatus::Forbidden`] instead of a silent retry loop: the watch ends, discovery
//! re-runs on [`CrdWatchConfig::forbidden_interval`] (so the registry still follows the cluster,
//! slowly) and the watch is attempted again then, once per interval.

use std::future::{self, Future};
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures::{Stream, StreamExt};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::core::PartialObjectMeta;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event, watcher};
use oxikube_domain::ErrorKind;
use oxikube_ports::CrdWatchStatus;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, sleep_until};
use tracing::{debug, warn};

use super::KubeDiscovery;
use crate::feed::error::feed_error;

/// Tuning for [`KubeDiscovery::watch_crds`].
#[derive(Debug, Clone)]
pub struct CrdWatchConfig {
    /// Quiet period after the last CRD change before discovery re-runs. Default 500 ms.
    pub debounce: Duration,
    /// Longest a continuous burst can delay the re-run. Default 3 s.
    pub max_wait: Duration,
    /// Wait before retrying a re-run that failed. Default 5 s.
    pub retry_delay: Duration,
    /// While the server refuses the watch (`Forbidden`): how often discovery re-runs and the
    /// watch is attempted again. Default 5 min.
    pub forbidden_interval: Duration,
}

impl Default for CrdWatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(500),
            max_wait: Duration::from_secs(3),
            retry_delay: Duration::from_secs(5),
            forbidden_interval: Duration::from_secs(300),
        }
    }
}

/// A running CRD watcher. The task is aborted when this handle is dropped.
#[derive(Debug)]
pub struct CrdWatch {
    task: JoinHandle<()>,
    attempts: Arc<AtomicU32>,
}

impl CrdWatch {
    /// Whether the task has ended (it only ends when the watch stream does, which kube's
    /// retrying watcher never does; this is for tests and diagnostics).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// How many times the watch was started: once, plus one per
    /// [`forbidden_interval`](CrdWatchConfig::forbidden_interval) while it is refused.
    pub fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::Relaxed)
    }
}

impl Drop for CrdWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// What one CRD watch connection reports.
pub(super) enum Signal {
    /// A CRD changed, or the (re)listing finished: discovery should re-run.
    Change,
    /// The server refused the watch; it will not heal by retrying.
    Forbidden(String),
}

impl KubeDiscovery {
    /// Starts watching CRDs; changes re-run discovery and publish to
    /// [`registry_changes`](Self::registry_changes) receivers. Must be called inside a Tokio
    /// runtime. Keep the handle alive for as long as the registry should follow the cluster.
    ///
    /// The initial listing also triggers one refresh: a CRD created between the caller's
    /// discovery and the watch starting would otherwise be missed. It publishes nothing when
    /// nothing changed. Likewise after a dropped connection is re-listed.
    ///
    /// A refused watch ends in [`CrdWatchStatus::Forbidden`] (see the module docs).
    pub fn watch_crds(&self, config: CrdWatchConfig) -> CrdWatch {
        let attempts = Arc::new(AtomicU32::new(0));
        let task = tokio::spawn({
            let discovery = self.clone();
            let attempts = attempts.clone();
            async move {
                let client = discovery.client.clone();
                watch_loop(
                    || {
                        // `Api<PartialObjectMeta<_>>` makes the watcher use metadata-only
                        // requests.
                        let api: Api<PartialObjectMeta<CustomResourceDefinition>> =
                            Api::all(client.clone());
                        crd_signals(watcher(api, watcher::Config::default()).default_backoff())
                    },
                    config,
                    attempts,
                    || discovery.refresh_after_crd_change(),
                    |status| discovery.set_crd_watch_status(status),
                )
                .await;
            }
        });
        CrdWatch { task, attempts }
    }
}

/// Maps the watcher's events to [`Signal`]s: `Forbidden` for a refusal that will not heal,
/// `Change` for CRD changes and finished listings; transient errors are logged and the backoff
/// wrapper retries them.
fn crd_signals<S>(events: S) -> impl Stream<Item = Signal>
where
    S: Stream<Item = Result<Event<PartialObjectMeta<CustomResourceDefinition>>, watcher::Error>>,
{
    events.filter_map(|event| async move {
        match event {
            // The initial listing's `InitApply`s are not changes; `InitDone` is the one signal
            // for "the listing finished", see `watch_crds`.
            Ok(Event::Apply(_) | Event::Delete(_) | Event::InitDone) => Some(Signal::Change),
            Ok(Event::Init | Event::InitApply(_)) => None,
            Err(err) => {
                let mapped = feed_error(&err);
                if mapped.kind() == ErrorKind::Forbidden && !mapped.is_retryable() {
                    return Some(Signal::Forbidden(mapped.message().to_owned()));
                }
                warn!(error = %err, "discovery: CRD watch error, retrying with backoff");
                None
            }
        }
    })
}

/// Runs one watch connection after another: normally one for ever. When a connection ends with
/// `Signal::Forbidden`, reports the status, re-runs discovery (the fallback for users who
/// cannot watch CRDs), waits `config.forbidden_interval` and tries again. Reports `Watching`
/// whenever a connection delivers a signal. Returns when a connection ends without a refusal.
pub(super) async fn watch_loop<M, S, R, Fut>(
    mut connect: M,
    config: CrdWatchConfig,
    attempts: Arc<AtomicU32>,
    mut refresh: R,
    status: impl Fn(CrdWatchStatus),
) where
    M: FnMut() -> S,
    S: Stream<Item = Signal>,
    R: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    loop {
        attempts.fetch_add(1, Ordering::Relaxed);
        let mut refused = None;
        let signals = connect()
            .take_while(|signal| {
                let proceed = match signal {
                    Signal::Forbidden(reason) => {
                        refused = Some(reason.clone());
                        false
                    }
                    Signal::Change => {
                        status(CrdWatchStatus::Watching);
                        true
                    }
                };
                future::ready(proceed)
            })
            .map(|_| ());
        debounce_refresh(signals, config.clone(), &mut refresh).await;
        let Some(reason) = refused else {
            return;
        };
        warn!(
            %reason,
            interval_s = config.forbidden_interval.as_secs(),
            "discovery: not allowed to watch CRDs; re-discovering on an interval instead"
        );
        status(CrdWatchStatus::Forbidden { reason });
        refresh().await;
        sleep(config.forbidden_interval).await;
    }
}

/// Runs `refresh` once per burst of `signals`: `config.debounce` after the last signal, at most
/// `config.max_wait` after the first. A refresh returning `false` is retried after
/// `config.retry_delay`. Returns when `signals` ends.
pub(super) async fn debounce_refresh<S, R, Fut>(signals: S, config: CrdWatchConfig, mut refresh: R)
where
    S: Stream<Item = ()>,
    R: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let mut signals = pin!(signals);
    // When the current burst started, and when the refresh is due.
    let mut burst_start: Option<Instant> = None;
    let mut due: Option<Instant> = None;
    loop {
        let signal = match due {
            None => signals.next().await,
            Some(at) => {
                tokio::select! {
                    signal = signals.next() => signal,
                    () = sleep_until(at) => {
                        burst_start = None;
                        due = if refresh().await {
                            None
                        } else {
                            Some(Instant::now() + config.retry_delay)
                        };
                        continue;
                    }
                }
            }
        };
        if signal.is_none() {
            debug!("discovery: CRD signal stream ended");
            return;
        }
        let now = Instant::now();
        let start = *burst_start.get_or_insert(now);
        due = Some((now + config.debounce).min(start + config.max_wait));
    }
}
