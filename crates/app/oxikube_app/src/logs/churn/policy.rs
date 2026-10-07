//! [`ReconnectPolicy`]: what a session does when its stream breaks, and how long it waits.

use std::hash::{Hash as _, Hasher as _};
use std::time::Duration;

/// Reconnects a session tries in a row when the setting says nothing (`logs.reconnect_retries`).
pub const DEFAULT_RECONNECT_RETRIES: u32 = 5;
/// Most reconnects a session may be set to try in a row.
pub const MAX_RECONNECT_RETRIES: u32 = 50;

/// Clamps a `logs.reconnect_retries` value to `0..=`[`MAX_RECONNECT_RETRIES`] (0 never
/// reconnects: a broken stream is `Failed` at once).
pub fn clamp_reconnect_retries(retries: u32) -> u32 {
    retries.min(MAX_RECONNECT_RETRIES)
}

/// What a session does when its stream breaks (a transport error, an API-server timeout, a
/// followed stream that closed while its pod still runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    /// The session ends (`Ended` or `Failed`) with the stream.
    Never,
    /// The session reopens the stream after a pause that doubles with each failure in a row
    /// (with jitter), from a little before the last line it received, and drops the replayed
    /// lines. After [`Backoff::max_retries`] failures in a row it is `Failed`.
    Backoff(Backoff),
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self::Backoff(Backoff::default())
    }
}

impl ReconnectPolicy {
    /// The backoff of [`ReconnectPolicy::Backoff`], `None` for [`ReconnectPolicy::Never`].
    pub fn backoff(self) -> Option<Backoff> {
        match self {
            Self::Never => None,
            Self::Backoff(backoff) => Some(backoff),
        }
    }
}

/// The timing of [`ReconnectPolicy::Backoff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Failures in a row before the session gives up (`logs.reconnect_retries`; the service's
    /// [`set_reconnect_retries`](crate::logs::LogService::set_reconnect_retries) changes it for
    /// open sessions). A stream that delivers a line, or stays open for [`stable`](Self::stable),
    /// starts the count again.
    pub max_retries: u32,
    /// The pause before the first reconnect.
    pub initial: Duration,
    /// The longest pause.
    pub max: Duration,
    /// How far before the last line received a reopened stream starts (`sinceTime`); the lines
    /// of that overlap that were already read are dropped.
    pub overlap: Duration,
    /// The pause between attempts to open a container that is still waiting to start (a pod of a
    /// rollout that exists before its containers do).
    pub start_wait: Duration,
    /// Attempts to open a container that is waiting to start before the session gives up.
    pub start_attempts: u32,
    /// How long a stream must stay open to count as healthy even when it brought no new line (a
    /// quiet pod behind a proxy or an API server that closes idle streams): its close then starts
    /// the failure count again rather than adding to it.
    pub stable: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_RECONNECT_RETRIES,
            initial: Duration::from_millis(500),
            max: Duration::from_secs(30),
            overlap: Duration::from_secs(2),
            start_wait: Duration::from_secs(1),
            start_attempts: 120,
            stable: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    /// The pause before reconnect `attempt` (1 for the first): `initial` doubled per attempt,
    /// at most `max`, plus up to a quarter more of jitter. The jitter is a hash of `salt` and
    /// the attempt, so sessions that broke together (an API-server restart) do not reconnect
    /// together, and a test gets the same pause every run.
    pub fn delay(&self, attempt: u32, salt: u64) -> Duration {
        let doublings = attempt.saturating_sub(1).min(16);
        let base = self.initial.saturating_mul(1 << doublings).min(self.max);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (salt, attempt).hash(&mut hasher);
        let quarter = base / 4;
        let nanos = u64::try_from(quarter.as_nanos()).unwrap_or(u64::MAX);
        let jitter = if nanos == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(hasher.finish() % (nanos + 1))
        };
        base + jitter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_doubles_up_to_the_cap_with_at_most_a_quarter_of_jitter() {
        let backoff = Backoff::default();
        let mut previous = Duration::ZERO;
        for attempt in 1..=12 {
            let base = (backoff.initial * 2u32.pow(attempt - 1)).min(backoff.max);
            let delay = backoff.delay(attempt, 7);
            assert!(
                delay >= base && delay <= base + base / 4,
                "{attempt}: {delay:?}"
            );
            assert!(
                delay + base / 4 >= previous,
                "never shorter than the last base"
            );
            previous = delay;
        }
        assert!(backoff.delay(40, 7) <= backoff.max + backoff.max / 4);
    }

    #[test]
    fn the_jitter_is_deterministic_per_salt_and_spreads_sessions() {
        let backoff = Backoff::default();
        assert_eq!(backoff.delay(3, 42), backoff.delay(3, 42));
        let spread: std::collections::HashSet<Duration> =
            (0..20).map(|salt| backoff.delay(3, salt)).collect();
        assert!(spread.len() > 10, "sessions that broke together spread out");
    }

    #[test]
    fn the_retry_setting_is_clamped() {
        assert_eq!(clamp_reconnect_retries(0), 0);
        assert_eq!(clamp_reconnect_retries(7), 7);
        assert_eq!(clamp_reconnect_retries(10_000), MAX_RECONNECT_RETRIES);
    }
}
