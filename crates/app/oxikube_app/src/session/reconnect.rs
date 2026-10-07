//! Automatic reconnect after the connection failed for a reason that may pass (E06-F440).
//!
//! The adapter's liveness loop gives up with [`HealthSignal::Failed`] after a few failed probes
//! (a VPN flap, a laptop waking from sleep) and the session goes to `Error`. The domain has no
//! `Error -> Ready` edge, so without help the user would have to press Retry. When the failure's
//! cause is transient ([`is_transient`]: `Network`, `Timeout` or a retryable `Auth`) the manager
//! therefore schedules a reconnect itself:
//!
//! ```text
//! Ready/Degraded --Failed (transient)--> Error --delay(1)--> Connecting --ok--> Ready
//!                                          ^                     │
//!                                          └── delay(n+1) ───────┘ transient failure
//! ```
//!
//! * The delays are the [`SessionManagerConfig::auto_reconnect`](super::SessionManagerConfig)
//!   policy (capped exponential backoff on the injected clock). Each attempt is an ordinary
//!   connect (its own short retries included); the adapter drops its pooled client before it
//!   reports `Failed`, so the attempt builds a fresh one and a new liveness loop starts with the
//!   new connection.
//! * It stops at `Ready`, at `AuthRequired` (the connect hit a 401: the user has to sign in), at
//!   an `Error` whose cause is permanent (a rejected certificate, a context that left the
//!   kubeconfig), and when the policy's attempts run out.
//! * A permanent failure is never retried: a non-retryable `Auth` goes straight to
//!   `AuthRequired`, anything else stays in `Error` ([`Shared::on_health`]).
//! * The user takes over at any time: a connect, reconnect, disconnect or close of the session
//!   cancels the schedule (and the task). The session's snapshot says what is planned
//!   ([`ClusterSession::auto_reconnect`](super::ClusterSession::auto_reconnect)), so the connect
//!   view can say that Oxikube is retrying by itself.
//!
//! The task needs a Tokio runtime (the adapter reports health from one); without one nothing is
//! scheduled and the session stays in `Error`, as before.

use std::sync::{Arc, Weak};
use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::SessionPhase;
use oxikube_ports::ClockPort;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use super::config::RetryPolicy;
use super::connect::Origin;
use super::entry::Entry;
use super::manager::Shared;

/// A reconnect the manager will make by itself: the session is in `Error` (or `Connecting` for
/// the attempt) after a transient failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoReconnect {
    /// Which automatic attempt is next (or running), from 1.
    pub attempt: u32,
    /// How long after the last failure that attempt starts.
    pub delay: Duration,
}

/// The schedule of one session, kept in its entry while it lasts.
pub(super) struct Reconnecting {
    pub(super) plan: AutoReconnect,
    /// Tells this schedule's task from an older one's.
    epoch: u64,
    _task: ReconnectTask,
}

/// The task behind a schedule; aborted when dropped, except from inside itself (it is ending
/// anyway, and aborting the running task would cancel the attempt it is finishing).
struct ReconnectTask(JoinHandle<()>);

impl Drop for ReconnectTask {
    fn drop(&mut self) {
        if tokio::task::try_id() != Some(self.0.id()) {
            self.0.abort();
        }
    }
}

/// Whether a failure with this cause may pass by itself, so reconnecting is worth it.
pub(super) fn is_transient(kind: ErrorKind, retryable: bool) -> bool {
    retryable
        && matches!(
            kind,
            ErrorKind::Network | ErrorKind::Timeout | ErrorKind::Auth
        )
}

impl Entry {
    /// After an automatic attempt ended: plans the next one when the attempt failed for a
    /// `transient` reason and the policy allows another, else ends the schedule. No-op when no
    /// schedule is running (the attempt was the user's).
    pub(super) fn after_attempt(&mut self, transient: bool, policy: Option<RetryPolicy>) {
        let Some(reconnecting) = self.reconnecting.as_mut() else {
            return;
        };
        let next = reconnecting.plan.attempt.saturating_add(1);
        match policy.filter(|p| transient && p.retries_after(next - 1)) {
            Some(policy) => {
                reconnecting.plan = AutoReconnect {
                    attempt: next,
                    delay: policy.delay(next),
                };
            }
            None => self.reconnecting = None,
        }
    }

    /// Whether the schedule `epoch` is still the session's and its attempt may start now.
    pub(super) fn is_due(&self, epoch: u64) -> bool {
        self.phase() == SessionPhase::Error
            && self.reconnecting.as_ref().is_some_and(|r| r.epoch == epoch)
    }

    /// The delay before the next attempt of schedule `epoch`, if it goes on.
    fn next_delay(&self, epoch: u64) -> Option<Duration> {
        self.is_due(epoch)
            .then(|| self.reconnecting.as_ref().map(|r| r.plan.delay))
            .flatten()
    }
}

impl Shared {
    /// Starts reconnecting `e` (just moved to `Error` by a transient failure). Returns whether a
    /// schedule started: not when automatic reconnects are off or no Tokio runtime is running.
    pub(super) fn schedule_reconnect(self: &Arc<Self>, e: &mut Entry) -> bool {
        let Some(policy) = self.config.auto_reconnect else {
            return false;
        };
        let Ok(handle) = Handle::try_current() else {
            tracing::debug!(cluster = %e.id, "no runtime; the session stays in Error");
            return false;
        };
        let plan = AutoReconnect {
            attempt: 1,
            delay: policy.delay(1),
        };
        e.reconnect_epoch += 1;
        let epoch = e.reconnect_epoch;
        let task = handle.spawn(run(
            Arc::downgrade(self),
            self.clock.clone(),
            e.id.clone(),
            epoch,
            plan.delay,
        ));
        tracing::info!(cluster = %e.id, delay = ?plan.delay, "connection lost; reconnecting");
        e.reconnecting = Some(Reconnecting {
            plan,
            epoch,
            _task: ReconnectTask(task),
        });
        true
    }
}

/// The schedule: wait, connect, and again after a transient failure, until the entry says stop.
/// Holds the manager weakly between attempts, so a dropped manager ends it.
async fn run(
    shared: Weak<Shared>,
    clock: Arc<dyn ClockPort>,
    cluster: ClusterId,
    epoch: u64,
    mut delay: Duration,
) {
    loop {
        clock.sleep(delay).await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let Some(entry) = shared.entry(&cluster) else {
            return;
        };
        shared
            .run_connect(entry.clone(), None, Origin::Auto(epoch))
            .await;
        let next = {
            let e = entry.lock();
            if e.phase() == SessionPhase::Ready {
                tracing::info!(%cluster, "reconnected");
            }
            e.next_delay(epoch)
        };
        let Some(next) = next else {
            return;
        };
        delay = next;
    }
}
