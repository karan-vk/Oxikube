//! The manager's vocabulary: [`ForwardId`], the [`ForwardInfo`] rows it lists, the
//! [`ForwardConfig`] it follows and the [`StartError`] a start can fail with.

use std::net::SocketAddr;

use oxikube_domain::OxiError;
use oxikube_domain::portforward::ForwardSpec;
use oxikube_ports::{ForwardStatsSnapshot, PortForwardError};

use super::state::{Backoff, ForwardState};

/// A forward's id in the manager: unique for the manager's life, stable across reconnects and
/// restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ForwardId(pub u64);

impl std::fmt::Display for ForwardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One forward as the manager lists it (a copy; read again after a change).
#[derive(Debug, Clone, PartialEq)]
pub struct ForwardInfo {
    /// The forward's id.
    pub id: ForwardId,
    /// What it forwards.
    pub spec: ForwardSpec,
    /// Where it is in its life.
    pub state: ForwardState,
    /// The local address it listens on (kept while it reconnects, so URLs stay valid); `None`
    /// before the first bind.
    pub local_addr: Option<SocketAddr>,
    /// The pod it last served from.
    pub pod: Option<String>,
    /// The last error one connection hit while the forward kept running (a refused port).
    pub last_error: Option<String>,
    /// Traffic since the forward was started (summed across reconnects), sampled when listed.
    pub stats: ForwardStatsSnapshot,
}

impl ForwardInfo {
    /// `pod/web 8080:80` or `service/api 0:http`: how the forward is named to the user.
    pub fn label(&self) -> String {
        label(&self.spec, self.local_addr)
    }
}

/// `kind/name local:remote`.
pub(crate) fn label(spec: &ForwardSpec, local_addr: Option<SocketAddr>) -> String {
    let local = local_addr.map_or(spec.local_port, |a| a.port());
    format!(
        "{}/{} {local}:{}",
        spec.target.gvk.kind.to_ascii_lowercase(),
        spec.target.name,
        spec.remote_port
    )
}

/// What the manager follows. In-code defaults until the `portforward.*` settings (E15-S08).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ForwardConfig {
    /// Whether forwards may start on a read-only cluster (`portforward.allowed_in_readonly`,
    /// default `false`).
    pub allowed_in_readonly: bool,
    /// The delays between reconnect attempts.
    pub backoff: Backoff,
}

/// Why a forward did not start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// The cluster is read-only and `portforward.allowed_in_readonly` is off.
    #[error(
        "{0} is read-only: port-forwards are off (set portforward.allowed_in_readonly to allow them)"
    )]
    ReadOnly(String),
    /// The cluster is not connected.
    #[error("{0} is not connected")]
    NotConnected(String),
    /// The target is not a namespaced core `Pod` or `Service`.
    #[error("only pods and services can be forwarded, not {0}")]
    Unsupported(String),
    /// No forward has this id.
    #[error("no port-forward {0}")]
    Unknown(ForwardId),
    /// The forward started but failed (the port's error, with its suggestion).
    #[error("{0}")]
    Forward(PortForwardError),
    /// The forward failed for a reason the port did not report (no such port on the target).
    #[error("{0}")]
    Failed(String),
}

impl StartError {
    /// What to do about it, when there is a hint (the port's suggestion, or the setting).
    pub fn suggestion(&self) -> Option<String> {
        match self {
            Self::Forward(err) => err.suggestion(),
            Self::ReadOnly(_) => {
                Some("Lift read-only mode, or set portforward.allowed_in_readonly.".to_owned())
            }
            _ => None,
        }
    }
}

impl From<StartError> for OxiError {
    fn from(err: StartError) -> Self {
        let message = match err.suggestion() {
            Some(hint) => format!("{err}. {hint}"),
            None => err.to_string(),
        };
        match err {
            StartError::ReadOnly(_) => OxiError::forbidden(message),
            StartError::NotConnected(_) | StartError::Unknown(_) => OxiError::not_found(message),
            StartError::Unsupported(_) | StartError::Failed(_) => OxiError::validation(message),
            StartError::Forward(inner) => OxiError::new(OxiError::from(inner).kind(), message),
        }
    }
}
