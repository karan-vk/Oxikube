//! Tuning for the [`ClusterSessionManager`](super::ClusterSessionManager) and the
//! per-session options a caller opens a session with.

use std::time::Duration;

use oxikube_domain::ClusterColour;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::ExecInteractivity;

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
    /// Capacity of the update broadcast. A subscriber that falls this far behind gets
    /// a [`SessionLagged`](super::SessionLagged) and should re-read
    /// [`sessions`](super::ClusterSessionManager::sessions).
    pub update_capacity: usize,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            retry: RetryPolicy::default(),
            update_capacity: 256,
        }
    }
}

/// The user-controlled fields a session starts with (per-cluster settings, E06-S08;
/// restored state, E06-S11). The defaults are: all namespaces, writable, no colour, exec
/// plugins may not prompt.
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
