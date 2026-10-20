//! The restart state machine of one forward, pure: [`ForwardState`], the [`ForwardEvent`]s that
//! move it, [`step`] and the [`Backoff`] between reconnect attempts.
//!
//! ```text
//! Starting ──connected──▶ Active ──pod gone / forward ended──▶ Reconnecting{1, 0 s}
//!    │                      ▲                                     │   │ transient failure:
//!    │ permanent            └──────────── connected ──────────────┘   │ Reconnecting{n+1, backoff}
//!    ▼                                                                ▼
//! Failed ◀──────────── permanent failure, or attempts exhausted ──────┘
//! Active / Reconnecting / Starting ──user stop──▶ Stopped;  any ──user restart──▶ Starting
//! ```
//!
//! The first reconnect after a drop is immediate (a pod replaced by a rollout is re-resolved the
//! moment the watch says so); the backoff only spaces out repeated failures.

use std::net::SocketAddr;
use std::time::Duration;

use oxikube_domain::notification::ForwardPhase;
use oxikube_ports::PortForwardError;

/// Why a forward is [`Failed`](ForwardState::Failed): the port's error when there is one, so the
/// dialog can show its [`suggestion`](PortForwardError::suggestion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardFailure {
    /// What went wrong, in one line for the UI.
    pub message: String,
    /// The port's structured error, when the failure came from it.
    pub error: Option<PortForwardError>,
}

impl ForwardFailure {
    /// A failure from the port's error.
    pub fn from_error(error: PortForwardError) -> Self {
        Self {
            message: error.to_string(),
            error: Some(error),
        }
    }

    /// A failure with only a message (the target cannot be resolved, the feed is forbidden).
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error: None,
        }
    }

    /// What to do about it, when the port's error has a hint.
    pub fn suggestion(&self) -> Option<String> {
        self.error.as_ref().and_then(PortForwardError::suggestion)
    }
}

/// Where a forward is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardState {
    /// Resolving the target and binding the listener.
    Starting,
    /// Listening on `local_addr` and bridging connections to `pod`.
    Active {
        /// The pod serving the forward.
        pod: String,
        /// The bound local address.
        local_addr: SocketAddr,
    },
    /// The forward dropped; the next attempt is `attempt`, `delay` after the drop or the last
    /// failure. The local port stays the same across attempts.
    Reconnecting {
        /// 1 for the first attempt after the drop.
        attempt: u32,
        /// How long until that attempt.
        delay: Duration,
        /// Why the forward is not active, in one line.
        reason: String,
    },
    /// Not running and not retrying: a permanent error or too many attempts. Needs a restart.
    Failed(ForwardFailure),
    /// Stopped by the user; listed until removed.
    Stopped,
}

impl ForwardState {
    /// The phase the notification centre knows.
    pub fn phase(&self) -> ForwardPhase {
        match self {
            Self::Starting => ForwardPhase::Starting,
            Self::Active { .. } => ForwardPhase::Active,
            Self::Reconnecting { .. } => ForwardPhase::Reconnecting,
            Self::Failed(_) => ForwardPhase::Failed,
            Self::Stopped => ForwardPhase::Stopped,
        }
    }

    /// Whether a supervisor runs for this state (it is neither failed nor stopped).
    pub fn is_running(&self) -> bool {
        !matches!(self, Self::Failed(_) | Self::Stopped)
    }

    /// One line on why the forward is not active, if it is not.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Reconnecting { reason, .. } => Some(reason),
            Self::Failed(failure) => Some(&failure.message),
            Self::Starting | Self::Active { .. } | Self::Stopped => None,
        }
    }
}

/// What happened to a forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardEvent {
    /// The listener is bound and the pod resolved.
    Connected {
        /// The pod serving the forward.
        pod: String,
        /// The bound local address.
        local_addr: SocketAddr,
    },
    /// A start or the running forward failed with `error`.
    Lost(PortForwardError),
    /// The target cannot be used: `permanent` failures (no such port) need the user.
    Unresolvable {
        /// Why, in one line.
        reason: String,
        /// Whether retrying cannot help.
        permanent: bool,
    },
    /// The watch says the pod went away or a new one is ready: try again now.
    TargetChanged {
        /// Why, in one line.
        reason: String,
    },
    /// The user stopped the forward.
    Stop,
    /// The user restarted the forward.
    Restart,
}

/// The delays between reconnect attempts: none before the first, then `initial`, doubling up to
/// `cap`, each spread by up to `jitter` (a fraction) either way; `max_attempts` before giving up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    /// The delay before the second attempt.
    pub initial: Duration,
    /// The longest delay.
    pub cap: Duration,
    /// The spread, as a fraction of the delay (`0.2` is ±20 %).
    pub jitter: f64,
    /// The attempts after a drop before the forward fails.
    pub max_attempts: u32,
}

impl Default for Backoff {
    /// 0, 0.5 s, 1 s, 2 s, ... 15 s, ±20 %, 12 attempts (about 1.5 minutes).
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(500),
            cap: Duration::from_secs(15),
            jitter: 0.2,
            max_attempts: 12,
        }
    }
}

impl Backoff {
    /// The delay before `attempt` (1-based). `unit` in `[0, 1)` picks the point in the jitter
    /// range (`0.5` is none), so the function stays pure.
    pub fn delay(&self, attempt: u32, unit: f64) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }
        let doublings = (attempt - 2).min(16);
        let base = self.initial.saturating_mul(1 << doublings).min(self.cap);
        let spread = (unit.clamp(0.0, 1.0) * 2.0 - 1.0) * self.jitter.clamp(0.0, 1.0);
        base.mul_f64(1.0 + spread).min(self.cap)
    }
}

/// Whether `error` needs the user rather than a retry, in `state`.
///
/// The port's [`is_retryable`](PortForwardError::is_retryable) kinds retry; so does the local
/// port being busy while reconnecting (the previous listener may still be closing). Everything
/// else (forbidden, the port closed, a privileged or busy port at the first start) is permanent.
pub fn is_permanent(error: &PortForwardError, state: &ForwardState) -> bool {
    if error.is_retryable() {
        return false;
    }
    let reconnecting = matches!(state, ForwardState::Reconnecting { .. });
    !matches!(
        (error, reconnecting),
        (
            PortForwardError::BindFailed {
                kind: std::io::ErrorKind::AddrInUse,
                ..
            },
            true
        )
    )
}

/// The state after `event`. `unit` feeds the backoff's jitter (see [`Backoff::delay`]).
pub fn step(
    state: &ForwardState,
    event: ForwardEvent,
    backoff: &Backoff,
    unit: f64,
) -> ForwardState {
    use ForwardState as S;
    let retry = |attempt: u32, reason: String| {
        if attempt > backoff.max_attempts {
            S::Failed(ForwardFailure::message(format!(
                "gave up after {} attempts: {reason}",
                backoff.max_attempts
            )))
        } else {
            S::Reconnecting {
                attempt,
                delay: backoff.delay(attempt, unit),
                reason,
            }
        }
    };
    let next_attempt = match state {
        S::Reconnecting { attempt, .. } => attempt + 1,
        _ => 1,
    };
    match (state, event) {
        (_, ForwardEvent::Restart) => S::Starting,
        (S::Failed(_) | S::Stopped, _) => state.clone(),
        (_, ForwardEvent::Stop) => S::Stopped,
        (_, ForwardEvent::Connected { pod, local_addr }) => S::Active { pod, local_addr },
        (_, ForwardEvent::Lost(error)) if is_permanent(&error, state) => {
            S::Failed(ForwardFailure::from_error(error))
        }
        (_, ForwardEvent::Lost(error)) => retry(next_attempt, error.to_string()),
        (_, ForwardEvent::Unresolvable { reason, permanent }) if permanent => {
            S::Failed(ForwardFailure::message(reason))
        }
        (_, ForwardEvent::Unresolvable { reason, .. }) => retry(next_attempt, reason),
        // A watch event: try now, without counting an attempt.
        (S::Reconnecting { attempt, .. }, ForwardEvent::TargetChanged { reason }) => {
            S::Reconnecting {
                attempt: *attempt,
                delay: Duration::ZERO,
                reason,
            }
        }
        (S::Active { .. }, ForwardEvent::TargetChanged { reason }) => S::Reconnecting {
            attempt: 1,
            delay: Duration::ZERO,
            reason,
        },
        (S::Starting, ForwardEvent::TargetChanged { .. }) => S::Starting,
    }
}
