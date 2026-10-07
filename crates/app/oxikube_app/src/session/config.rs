//! Tuning for the [`ClusterSessionManager`](super::ClusterSessionManager) and the
//! per-session options a caller opens a session with.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ClusterColour;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{ClusterPrefs, ExecInteractivity};

/// How a connect attempt retries transient failures.
///
/// Only retryable errors ([`OxiError::is_retryable`](oxikube_domain::OxiError::is_retryable):
/// network, timeout) are retried, and never `Auth` errors: those need the user. The delay
/// before retry `n` (1-based) is `initial_delay * factor^(n-1)`, capped at `max_delay`,
/// and is slept on the injected `ClockPort`, so tests run on virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts per `connect` call, including the first (at least 1).
    pub max_attempts: u32,
    /// Delay before the first retry.
    pub initial_delay: Duration,
    /// Upper bound for any delay.
    pub max_delay: Duration,
    /// Growth factor between retries.
    pub factor: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
            factor: 2,
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// The delay before retry number `retry` (1-based).
    pub fn delay(&self, retry: u32) -> Duration {
        let growth = self.factor.max(1).saturating_pow(retry.saturating_sub(1));
        self.initial_delay
            .saturating_mul(growth)
            .min(self.max_delay)
    }

    /// The automatic reconnect after a transient connection failure (E06-F440): first attempt
    /// 1 s after the failure, then 2, 4, 8 and 16 s, then every 30 s until the cluster answers
    /// again, the credentials are rejected, the failure turns permanent or the user steps in.
    /// 30 s matches the healthy probe interval, so a laptop that wakes from sleep is back
    /// within one probe interval of its network.
    pub fn auto_reconnect() -> Self {
        Self {
            max_attempts: u32::MAX,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            factor: 2,
        }
    }

    /// Whether another attempt follows `failed` failed attempts.
    pub(super) fn retries_after(&self, failed: u32) -> bool {
        failed < self.max_attempts.max(1)
    }
}

/// Manager-wide configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionManagerConfig {
    /// Retry policy for connect attempts.
    pub retry: RetryPolicy,
    /// The automatic reconnect after the connection failed for a transient reason (a
    /// `Network`, `Timeout` or retryable `Auth` health failure, E06-F440): `max_attempts`
    /// counts automatic reconnects (each an ordinary connect with its own `retry`), the delays
    /// are slept on the injected clock. `None` leaves the session in `Error` until the user
    /// retries.
    pub auto_reconnect: Option<RetryPolicy>,
    /// Capacity of the update broadcast. A subscriber that falls this far behind gets
    /// a [`SessionLagged`](super::SessionLagged) and should re-read
    /// [`sessions`](super::ClusterSessionManager::sessions).
    pub update_capacity: usize,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            retry: RetryPolicy::default(),
            auto_reconnect: Some(RetryPolicy::auto_reconnect()),
            update_capacity: 256,
        }
    }
}

/// The user-controlled fields a session starts with (per-cluster settings, E06-S08, see
/// [`SessionOptions::from_prefs`]; restored state, E06-S11). The defaults are: all
/// namespaces, writable, no colour, no display name, exec plugins may not prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionOptions {
    /// Which namespaces the session watches.
    pub namespace_selection: NamespaceSelection,
    /// Whether mutations are blocked (enforced by `MutationGuard`, E06-S02).
    pub read_only: bool,
    /// The cluster's accent colour.
    pub colour: Option<ClusterColour>,
    /// The exec credential plugin policy passed to the connector.
    pub exec_interactivity: ExecInteractivity,
    /// The name shown instead of the context name.
    pub display_name: Option<String>,
    /// The settings these options were derived from; later pushes of a
    /// [`ClusterPrefsTable`](oxikube_ports::ClusterPrefsTable) are applied relative to them.
    pub prefs: Arc<ClusterPrefs>,
}

impl SessionOptions {
    /// The options a session starts with under `prefs`.
    ///
    /// The namespace selection starts at the cluster's `default_namespace`, else at
    /// `kubeconfig_namespace` (the context's own), else all namespaces.
    pub fn from_prefs(prefs: &Arc<ClusterPrefs>, kubeconfig_namespace: Option<&str>) -> Self {
        let namespace = prefs.default_namespace.as_deref().or(kubeconfig_namespace);
        Self {
            namespace_selection: namespace
                .map(NamespaceSelection::single)
                .unwrap_or_default(),
            read_only: prefs.read_only,
            colour: prefs.colour,
            exec_interactivity: prefs.exec_interactivity,
            display_name: prefs.display_name.clone(),
            prefs: prefs.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_grow_and_cap() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1), Duration::from_millis(500));
        assert_eq!(policy.delay(2), Duration::from_secs(1));
        assert_eq!(policy.delay(3), Duration::from_secs(2));
        assert_eq!(policy.delay(10), Duration::from_secs(8));
        assert_eq!(policy.delay(u32::MAX), Duration::from_secs(8));
    }

    #[test]
    fn auto_reconnect_backs_off_to_the_probe_interval_and_keeps_going() {
        let policy = RetryPolicy::auto_reconnect();
        let delays: Vec<u64> = (1..=7).map(|n| policy.delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
        assert!(policy.retries_after(10_000));
        assert_eq!(SessionManagerConfig::default().auto_reconnect, Some(policy));
    }

    #[test]
    fn attempts_are_bounded_and_at_least_one() {
        let policy = RetryPolicy::default();
        assert!(policy.retries_after(1));
        assert!(policy.retries_after(2));
        assert!(!policy.retries_after(3));
        let zero = RetryPolicy {
            max_attempts: 0,
            ..policy
        };
        assert!(!zero.retries_after(1));
        assert!(!RetryPolicy::no_retry().retries_after(1));
    }
}
