//! [`ClusterConnectorPort`]: turn a kubeconfig context into a live set of cluster ports.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube` (a connector over its `ClientPool`, discovery, health
//! and data-plane adapters) and wired by `bins/oxikube`; `oxikube_testkit` ships
//! `FakeClusterConnectorPort`. The `ClusterSessionManager` (`oxikube_app::session`) is
//! the only caller: it never sees adapter types, only the [`ClusterPorts`] bundle.
//!
//! # Contract
//!
//! [`connect`](ClusterConnectorPort::connect) builds (or reuses) the client for the
//! context, applying the [`ExecInteractivity`] policy to exec credential plugins, and
//! returns a [`ClusterConnection`]. It does not run discovery or the capability probe;
//! the manager calls those through the bundle so it can classify their failures too.
//!
//! The adapter reports the connection's health through the [`HealthReporter`] in the
//! [`ConnectRequest`] (a liveness loop of its own; the manager does not spawn tasks).
//! Signals that arrive before the session is `Ready` are ignored by the manager.
//! Dropping the [`ClusterConnection`] (its [`ConnectionGuard`]) stops that loop and
//! releases everything the adapter keeps for the connection: feeds are torn down when
//! the session is.
//!
//! Credentials never cross this port: the adapter keeps tokens and exec output to
//! itself (non-negotiable 5), and error messages are redacted before they are built.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::SessionEvent;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use serde::{Deserialize, Serialize};

use crate::{
    AccessReviewPort, DescribePort, DiscoveryPort, ExecPort, LogPort, MetricsPort, PortForwardPort,
    ResourcePort, TableFeedPort, WarningPort,
};

/// How interactive an exec credential plugin may be (a per-cluster setting, E06-S08).
///
/// Ordered by permissiveness: `Never < IfAvailable < Always`. The adapter maps it onto
/// the kubeconfig's `interactiveMode` and refuses plugins that demand more.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecInteractivity {
    /// Plugins run without stdin and may not prompt. The GUI default.
    #[default]
    Never,
    /// Plugins may use a terminal when one exists.
    IfAvailable,
    /// Plugins that demand interaction are allowed.
    Always,
}

/// One health observation of a live connection.
///
/// Reasons are user-visible and must not contain secrets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HealthSignal {
    /// A probe (or feed) succeeded: the session is, or is again, `Ready`.
    Healthy,
    /// A probe (or feed) failed but the connection may recover: `Degraded`.
    Unhealthy,
    /// The connection is not coming back without a reconnect: `Error`.
    ///
    /// The cause tells the session manager what to do next (E06-F440): a transient one
    /// (`Network`, `Timeout`, a retryable `Auth`) is reconnected automatically with backoff;
    /// a non-retryable `Auth` (revoked or expired credentials) goes to `AuthRequired`;
    /// anything else stays in `Error` until the user retries. Build it from the error with
    /// [`HealthSignal::failed`].
    Failed {
        /// What went wrong: the error's `Display` (`"<kind label>: <message>"`).
        reason: String,
        /// The kind of the failure that ended the probing.
        kind: ErrorKind,
        /// Whether retrying may fix it ([`OxiError::is_retryable`]).
        retryable: bool,
    },
}

impl HealthSignal {
    /// `Failed` for `error`: its `Display` as the reason, its kind and retry flag as the cause.
    /// The error's message must already be redacted (adapters build it that way).
    pub fn failed(error: &OxiError) -> Self {
        HealthSignal::Failed {
            reason: error.to_string(),
            kind: error.kind(),
            retryable: error.is_retryable(),
        }
    }

    /// The session state-machine event this signal stands for, before the session manager
    /// routes a `Failed` by its cause (a non-retryable `Auth` becomes `AuthNeeded` there).
    pub fn to_session_event(&self) -> SessionEvent {
        match self {
            HealthSignal::Healthy => SessionEvent::Healthy,
            HealthSignal::Unhealthy => SessionEvent::Unhealthy,
            HealthSignal::Failed { reason, .. } => SessionEvent::Failed {
                reason: reason.clone(),
            },
        }
    }
}

/// Where an adapter sends [`HealthSignal`]s for one connection.
///
/// Implemented by the session manager. `report` is cheap and non-blocking (a short lock
/// and a channel send), so it may be called from any thread or task. A `Failed` report
/// may drop the [`ClusterConnection`] before `report` returns, so the adapter must not
/// hold a lock its [`ConnectionGuard`]'s drop needs while reporting.
pub trait HealthReporter: Send + Sync {
    /// Records one observation.
    fn report(&self, signal: HealthSignal);
}

/// What the manager asks the connector for.
#[derive(Clone)]
pub struct ConnectRequest {
    /// The catalog entry being connected.
    pub cluster: ClusterId,
    /// Its context name in the kubeconfig.
    pub context: ContextName,
    /// The exec credential plugin policy for this cluster.
    pub exec_interactivity: ExecInteractivity,
    /// Where to report the connection's health from now on.
    pub health: Arc<dyn HealthReporter>,
}

impl fmt::Debug for ConnectRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectRequest")
            .field("cluster", &self.cluster)
            .field("context", &self.context)
            .field("exec_interactivity", &self.exec_interactivity)
            .finish_non_exhaustive()
    }
}

/// The per-cluster ports of one connection.
///
/// Cheap to clone (every field is an `Arc`). `resources` includes the
/// [`ResourceWriter`](crate::ResourceWriter) half: **Mutating** methods on it are
/// reachable only through `MutationGuard` (non-negotiable 3), which is why
/// `oxikube_app` hands out the read half only.
#[derive(Clone)]
pub struct ClusterPorts {
    /// Read and write cluster objects.
    pub resources: Arc<dyn ResourcePort>,
    /// API discovery for this cluster.
    pub discovery: Arc<dyn DiscoveryPort>,
    /// Server-side Table feeds.
    pub tables: Arc<dyn TableFeedPort>,
    /// Container logs.
    pub logs: Arc<dyn LogPort>,
    /// Exec and attach.
    pub exec: Arc<dyn ExecPort>,
    /// Port-forwarding.
    pub port_forward: Arc<dyn PortForwardPort>,
    /// `metrics.k8s.io`.
    pub metrics: Arc<dyn MetricsPort>,
    /// What the user may do here.
    pub access: Arc<dyn AccessReviewPort>,
    /// The API server's `Warning:` response headers.
    pub warnings: Arc<dyn WarningPort>,
    /// `kubectl describe`-style text for one object (read-only).
    pub describe: Arc<dyn DescribePort>,
}

impl fmt::Debug for ClusterPorts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterPorts").finish_non_exhaustive()
    }
}

/// Keeps an adapter's per-connection resources alive (liveness loop, client lease).
/// Dropping it releases them.
#[derive(Default)]
pub struct ConnectionGuard(Option<Box<dyn Send + Sync>>);

impl ConnectionGuard {
    /// A guard owning `resources`; they are dropped with the guard.
    pub fn new(resources: impl Send + Sync + 'static) -> Self {
        Self(Some(Box::new(resources)))
    }

    /// A guard that owns nothing.
    pub fn none() -> Self {
        Self(None)
    }
}

impl fmt::Debug for ConnectionGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ConnectionGuard")
            .field(&self.0.is_some())
            .finish()
    }
}

/// A live connection: the ports bundle plus what keeps the adapter side alive.
#[derive(Debug)]
pub struct ClusterConnection {
    /// The per-cluster ports.
    pub ports: ClusterPorts,
    /// Released (and the health loop stopped) when the connection is dropped.
    pub guard: ConnectionGuard,
}

/// Connects to clusters by context.
///
/// # Effects
///
/// Builds or reuses a client and may run an exec credential plugin; never writes to the
/// cluster.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. The session
/// manager branches on the kind:
/// [`Auth`](oxikube_domain::ErrorKind::Auth) (exec plugin needs a login, 401, a plugin
/// that wants more interaction than [`ExecInteractivity`] allows) moves the session to
/// `AuthRequired` with the message as the reason;
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) are retried with backoff;
/// anything else ([`NotFound`](oxikube_domain::ErrorKind::NotFound) for a context that is
/// gone, [`Validation`](oxikube_domain::ErrorKind::Validation) for a broken kubeconfig
/// entry, a rejected certificate) moves it to `Error`.
#[async_trait]
pub trait ClusterConnectorPort: Send + Sync {
    /// Connects to `request.context`. Dropping the returned future cancels the attempt.
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn health_signals_map_onto_session_events() {
        assert_eq!(
            HealthSignal::Healthy.to_session_event(),
            SessionEvent::Healthy
        );
        assert_eq!(
            HealthSignal::Unhealthy.to_session_event(),
            SessionEvent::Unhealthy
        );
        assert_eq!(
            HealthSignal::failed(&OxiError::network("gone")).to_session_event(),
            SessionEvent::Failed {
                reason: "network error: gone".into()
            }
        );
    }

    #[test]
    fn a_failed_signal_carries_the_cause() {
        assert_eq!(
            HealthSignal::failed(&OxiError::auth("revoked", false)),
            HealthSignal::Failed {
                reason: "authentication failed: revoked".into(),
                kind: ErrorKind::Auth,
                retryable: false,
            }
        );
        let timeout = HealthSignal::failed(&OxiError::timeout("slow"));
        assert!(matches!(
            timeout,
            HealthSignal::Failed {
                kind: ErrorKind::Timeout,
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn exec_interactivity_is_ordered_and_defaults_to_never() {
        assert_eq!(ExecInteractivity::default(), ExecInteractivity::Never);
        assert!(ExecInteractivity::Never < ExecInteractivity::IfAvailable);
        assert!(ExecInteractivity::IfAvailable < ExecInteractivity::Always);
        assert_eq!(
            serde_json::to_string(&ExecInteractivity::IfAvailable).unwrap(),
            r#""if_available""#
        );
    }

    #[test]
    fn dropping_the_guard_releases_its_resources() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let guard = ConnectionGuard::new(Counted(dropped.clone()));
        assert_eq!(format!("{guard:?}"), "ConnectionGuard(true)");
        drop(guard);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            format!("{:?}", ConnectionGuard::none()),
            "ConnectionGuard(false)"
        );
    }
}
