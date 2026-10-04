//! The liveness loop: probe the apiserver periodically, with backoff after failures, and
//! report [`HealthEvent`]s.
//!
//! The loop knows nothing about kube clients. It is given a probe function (usually a
//! closure around [`probe_apiserver_version`]), runs it on a Tokio task, and sends events
//! to a channel; the session manager maps them onto `ClusterSessionState` with
//! [`HealthEvent::to_session_event`]. The failure policy lives in a pure state machine;
//! see [`HealthEvent`] and [`LivenessConfig`].
//!
//! # Cadence
//!
//! After a success the next probe is one `interval` away. After a failure it comes after
//! an exponential backoff (`backon`), never longer than `interval`. Both come from
//! `tokio::time`, so tests drive them with paused time.
//!
//! # Controls
//!
//! [`Liveness::pause`] and [`resume`](Liveness::resume) stop and restart probing (hidden
//! tabs; docs/PERFORMANCE.md rule 7), [`set_interval`](Liveness::set_interval) retunes it,
//! [`probe_now`](Liveness::probe_now) forces a probe, and dropping or
//! [`stop`](Liveness::stop)ping the handle aborts the task. The loop also ends by itself
//! after it emits [`HealthEvent::Failed`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};
use futures::FutureExt;
use kube::Client;
use oxikube_domain::{OxiError, OxiResult};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub use self::machine::HealthEvent;
use self::machine::HealthMachine;
use crate::auth::{CredentialRefresh, classify_with};

mod machine;

/// The shortest interval and probe timeout accepted; smaller values (including zero) are
/// raised to this so a bad setting cannot turn the loop into a busy loop.
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Tuning for [`Liveness::spawn`].
#[derive(Debug, Clone, Copy)]
pub struct LivenessConfig {
    /// Time between probes while healthy (at least [`MIN_INTERVAL`]). A modest default
    /// keeps idle CPU near zero.
    pub interval: Duration,
    /// A single probe that takes longer than this (at least [`MIN_INTERVAL`]) fails with
    /// a `Timeout` error.
    pub probe_timeout: Duration,
    /// This many failures in a row end the run with [`HealthEvent::Failed`].
    pub failure_threshold: u32,
    /// Delays between probes after a failure (jitter off by default; each delay is also
    /// capped at `interval`). The builder's own retry limit is ignored.
    pub backoff: ExponentialBuilder,
}

impl Default for LivenessConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(10),
            failure_threshold: 3,
            backoff: ExponentialBuilder::new()
                .with_min_delay(Duration::from_secs(2))
                .with_max_delay(Duration::from_secs(30))
                .with_factor(2.0)
                .without_max_times(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Settings {
    interval: Duration,
    paused: bool,
}

/// Handle to a running probe loop. Dropping it aborts the task.
#[derive(Debug)]
pub struct Liveness {
    settings: watch::Sender<Settings>,
    probe_now: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Liveness {
    /// Starts probing now (the first probe runs immediately) and returns the handle plus
    /// the event receiver. Must be called inside a Tokio runtime.
    ///
    /// Start the loop once the session is `Ready`: `Healthy` and `Unhealthy` are not legal
    /// session events while it is still `Connecting`.
    ///
    /// The receiver closes when the loop ends (after `Failed`, or when the handle is
    /// dropped). A slow receiver slows the loop down rather than losing events.
    pub fn spawn<P, Fut>(
        mut config: LivenessConfig,
        probe: P,
    ) -> (Liveness, mpsc::Receiver<HealthEvent>)
    where
        P: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = OxiResult<String>> + Send + 'static,
    {
        config.interval = config.interval.max(MIN_INTERVAL);
        config.probe_timeout = config.probe_timeout.max(MIN_INTERVAL);
        let (tx, rx) = mpsc::channel(32);
        let (settings, settings_rx) = watch::channel(Settings {
            interval: config.interval,
            paused: false,
        });
        let probe_now = Arc::new(Notify::new());
        let task = tokio::spawn(run(config, probe, settings_rx, probe_now.clone(), tx));
        (
            Liveness {
                settings,
                probe_now,
                task,
            },
            rx,
        )
    }

    /// Stops probing until [`resume`](Self::resume). A probe already in flight finishes.
    pub fn pause(&self) {
        self.settings.send_modify(|s| s.paused = true);
    }

    /// Resumes probing; the next probe runs immediately (or, if a probe is still in
    /// flight, as soon as it finishes). No effect when not paused.
    pub fn resume(&self) {
        self.settings.send_if_modified(|s| {
            if !s.paused {
                return false;
            }
            s.paused = false;
            // The loop may not have seen the pause at all (a pause and resume during one
            // in-flight probe coalesce in the watch channel), so ask for a probe
            // explicitly. A loop parked on the pause probes on waking anyway and drops
            // this request first. Requesting inside the closure stores the request
            // before the loop can wake, so it cannot outlive that probe.
            self.probe_now.notify_one();
            true
        });
    }

    /// Changes the healthy-state interval (raised to [`MIN_INTERVAL`] if smaller). Takes
    /// effect for the current wait.
    pub fn set_interval(&self, interval: Duration) {
        let interval = interval.max(MIN_INTERVAL);
        self.settings.send_modify(|s| s.interval = interval);
    }

    /// Runs a probe as soon as the loop is idle. While paused it has no effect: resuming
    /// probes at once regardless, and a request made while paused is not replayed after
    /// that probe.
    pub fn probe_now(&self) {
        self.probe_now.notify_one();
    }

    /// True once the loop has ended on its own.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Aborts the loop.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Liveness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run<P, Fut>(
    config: LivenessConfig,
    mut probe: P,
    mut settings: watch::Receiver<Settings>,
    probe_now: Arc<Notify>,
    tx: mpsc::Sender<HealthEvent>,
) where
    P: FnMut() -> Fut,
    Fut: Future<Output = OxiResult<String>>,
{
    let mut machine = HealthMachine::new(config.failure_threshold);
    let mut backoff = config.backoff.build();
    loop {
        if !wait_unpaused(&mut settings).await {
            return;
        }
        // The probe that starts now answers every `probe_now` made before it (including
        // any made while paused), so a stored request must not trigger a second one.
        drop_pending_request(&probe_now);
        let result = match tokio::time::timeout(config.probe_timeout, probe()).await {
            Ok(result) => result,
            Err(_) => Err(OxiError::timeout(format!(
                "the cluster did not answer within {}s",
                config.probe_timeout.as_secs().max(1)
            ))),
        };
        let healthy = result.is_ok();
        for event in machine.on_probe(result) {
            if tx.send(event).await.is_err() {
                return;
            }
        }
        if machine.is_done() {
            return;
        }
        let failure_delay = if healthy {
            backoff = config.backoff.build();
            None
        } else {
            Some(backoff.next().unwrap_or(config.backoff_max()))
        };
        if !wait_next(failure_delay, &mut settings, &probe_now).await {
            return;
        }
    }
}

impl LivenessConfig {
    /// Fallback delay if the backoff iterator ever ends (it does not by default).
    fn backoff_max(&self) -> Duration {
        self.interval
    }
}

/// Waits until probing is not paused. False when the handle is gone.
async fn wait_unpaused(settings: &mut watch::Receiver<Settings>) -> bool {
    settings.wait_for(|s| !s.paused).await.is_ok()
}

/// Consumes a `probe_now` request stored while nobody was waiting, if there is one.
fn drop_pending_request(probe_now: &Notify) {
    // Polling a fresh `Notified` once takes the stored permit, if any, and otherwise
    // leaves nothing registered when it is dropped.
    let _ = probe_now.notified().now_or_never();
}

/// Sleeps until the next probe is due: `failure_delay` (capped at the interval) after a
/// failure, else the interval. Returns early on `probe_now`, and re-evaluates when the
/// interval changes. While paused (including a pause that arrived during the previous
/// probe) it holds until resumed and then returns at once. False when the handle is gone.
async fn wait_next(
    failure_delay: Option<Duration>,
    settings: &mut watch::Receiver<Settings>,
    probe_now: &Notify,
) -> bool {
    let started = Instant::now();
    loop {
        let current = *settings.borrow_and_update();
        if current.paused {
            return wait_unpaused(settings).await;
        }
        let delay = failure_delay.map_or(current.interval, |d| d.min(current.interval));
        let deadline = started + delay;
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => return true,
            () = probe_now.notified() => return true,
            changed = settings.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
}

/// The real probe: `GET /version`, mapped through [`classify_with`].
///
/// `/version` is cheap and exercises DNS, TLS and authentication. A bad token gets a
/// 401 here even though anonymous users may read `/version`, because a presented but
/// invalid credential is rejected before the anonymous fallback. `/livez` and `/readyz`
/// would add control-plane internals Oxikube does not act on, so they are not used.
///
/// # Errors
///
/// The classified failure; wrap with [`retry_once_kube`](crate::auth::retry_once_kube)
/// at the call site if a client rebuild should be tried first.
pub async fn probe_apiserver_version(
    client: &Client,
    refresh: CredentialRefresh,
) -> OxiResult<String> {
    client
        .apiserver_version()
        .await
        .map(|info| info.git_version)
        .map_err(|e| classify_with(&e, refresh))
}

#[cfg(test)]
mod tests;
