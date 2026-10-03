//! Keeps the registry current by watching `CustomResourceDefinition`s.
//!
//! A metadata-only watch (names and versions are all that matter; the CRD schemas can be
//! megabytes) turns every CRD change into a signal. Signals are debounced (a `helm install` of
//! an operator creates dozens of CRDs in a burst) and then trigger one
//! [`KubeDiscovery::refresh`], which publishes the registry diff. When nothing changes the task
//! is parked on the watch connection: no timers, no polling.

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use futures::{Stream, StreamExt};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::core::PartialObjectMeta;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event, watcher};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, warn};

use super::KubeDiscovery;

/// Tuning for [`KubeDiscovery::watch_crds`].
#[derive(Debug, Clone)]
pub struct CrdWatchConfig {
    /// Quiet period after the last CRD change before discovery re-runs. Default 500 ms.
    pub debounce: Duration,
    /// Longest a continuous burst can delay the re-run. Default 3 s.
    pub max_wait: Duration,
    /// Wait before retrying a re-run that failed. Default 5 s.
    pub retry_delay: Duration,
}

impl Default for CrdWatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(500),
            max_wait: Duration::from_secs(3),
            retry_delay: Duration::from_secs(5),
        }
    }
}

/// A running CRD watcher. The task is aborted when this handle is dropped.
#[derive(Debug)]
pub struct CrdWatch {
    task: JoinHandle<()>,
}

impl CrdWatch {
    /// Whether the task has ended (it only ends when the watch stream does, which kube's
    /// retrying watcher never does; this is for tests and diagnostics).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for CrdWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl KubeDiscovery {
    /// Starts watching CRDs; changes re-run discovery and publish to
    /// [`subscribe`](Self::subscribe)rs. Must be called inside a Tokio runtime. Keep the handle
    /// alive for as long as the registry should follow the cluster.
    ///
    /// The initial listing also triggers one refresh: a CRD created between the caller's
    /// discovery and the watch starting would otherwise be missed. It publishes nothing when
    /// nothing changed. Likewise after a dropped connection is re-listed.
    pub fn watch_crds(&self, config: CrdWatchConfig) -> CrdWatch {
        let discovery = self.clone();
        // `Api<PartialObjectMeta<_>>` makes the watcher use metadata-only requests.
        let api: Api<PartialObjectMeta<CustomResourceDefinition>> = Api::all(self.client.clone());
        let task = tokio::spawn(async move {
            let signals = watcher(api, watcher::Config::default())
                .default_backoff()
                .filter_map(|event| async move {
                    match event {
                        // The initial listing's `InitApply`s are not changes; `InitDone` is
                        // the one signal for "the listing finished", see above.
                        Ok(Event::Apply(_) | Event::Delete(_) | Event::InitDone) => Some(()),
                        Ok(Event::Init | Event::InitApply(_)) => None,
                        Err(err) => {
                            warn!(error = %err, "discovery: CRD watch error, retrying with backoff");
                            None
                        }
                    }
                });
            debounce_refresh(signals, config, || async {
                match discovery.refresh().await {
                    Ok(_) => true,
                    Err(err) => {
                        warn!(error = %err, "discovery: refresh after CRD change failed");
                        false
                    }
                }
            })
            .await;
        });
        CrdWatch { task }
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
