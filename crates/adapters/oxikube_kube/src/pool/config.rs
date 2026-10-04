//! [`PoolConfig`]: the knobs applied to every client the pool builds.
//!
//! Defaults live here so a settings layer can feed the struct later without the
//! pool changing. Values are checked against kube 4.2 (`kube_client::Config`).

use std::time::Duration;

use super::eviction::EvictionPolicy;
use crate::auth::ExecInteractivePolicy;

/// Default TCP + TLS connect timeout. Shorter than kube's 30 s so an unreachable
/// cluster fails fast in the UI; the health probe (E03-S05) retries on its own.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default socket write timeout. Same as kube's own default (295 s).
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(295);

/// Default limit on building one client, which is mostly the first exec credential
/// plugin run (`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`). Long
/// enough for a slow token exchange, short enough that a hung plugin surfaces as an
/// error instead of a context that never connects.
pub const DEFAULT_EXEC_DEADLINE: Duration = Duration::from_secs(30);

/// How the client retries transient server failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetryMode {
    /// kube's `RetryPolicy::server_retry()`: retries 429, 503 and 504 with exponential
    /// backoff (5 ms to 1000 s, at most 15 attempts) and honours `Retry-After`.
    /// kube installs it when `Config::default_retry` is `true`
    /// (`kube_client::client::builder`, 4.2).
    #[default]
    ServerRetry,
    /// No automatic retries; the caller handles every failure.
    Disabled,
}

impl RetryMode {
    /// The value for `kube::Config::default_retry`.
    pub fn default_retry(self) -> bool {
        matches!(self, RetryMode::ServerRetry)
    }
}

/// Settings applied to every client in a [`ClientPool`](super::ClientPool).
///
/// # Why there is no read timeout
///
/// `read_timeout` defaults to `None` on purpose. The pool shares one client per
/// context across every caller, including long-lived streams: exec, attach,
/// port-forward, log follows and watches. A read timeout on the shared client
/// would cut any of those streams that is quiet for longer than the timeout (a
/// shell nobody types in, a watch on a quiet namespace). Watches already have a
/// watcher-level idle timeout, and unary requests get per-call deadlines from
/// their callers. Set a value only if it is longer than any stream should idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    /// TCP + TLS connect timeout. `None` waits forever.
    pub connect_timeout: Option<Duration>,
    /// Response read timeout. `None` by design; see the type docs.
    pub read_timeout: Option<Duration>,
    /// Socket write timeout. `None` waits forever.
    pub write_timeout: Option<Duration>,
    /// Retry behaviour for transient server failures.
    pub retry: RetryMode,
    /// The most interaction an exec credential plugin may ask for (E03-S04).
    /// Defaults to [`ExecInteractivePolicy::Never`]: a GUI cannot answer prompts.
    pub exec_policy: ExecInteractivePolicy,
    /// Limit on building one client, exec plugin included. kube 4.2 runs an exec
    /// plugin three times per build, one after another (expiry, TLS client identity,
    /// auth layer), so this bounds all three runs. A build that overruns fails with
    /// [`Timeout`](oxikube_domain::ErrorKind::Timeout); it keeps running on the
    /// blocking pool (a plugin process cannot be cancelled) and the next `get` waits
    /// on it again instead of starting another plugin.
    pub exec_deadline: Duration,
    /// When idle clients are dropped.
    pub eviction: EvictionPolicy,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Some(DEFAULT_CONNECT_TIMEOUT),
            read_timeout: None,
            write_timeout: Some(DEFAULT_WRITE_TIMEOUT),
            retry: RetryMode::ServerRetry,
            exec_policy: ExecInteractivePolicy::default(),
            exec_deadline: DEFAULT_EXEC_DEADLINE,
            eviction: EvictionPolicy::default(),
        }
    }
}
