//! The manager's vocabulary: [`ForwardId`], the [`ForwardInfo`] rows it lists, the
//! [`ForwardConfig`] it follows and the [`StartError`] a start can fail with.

use std::net::{IpAddr, SocketAddr};

use oxikube_domain::portforward::{ForwardSpec, PortRange};
use oxikube_domain::{ErrorKind, OxiError};
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
    /// When the forward was started (or last restarted), on the manager's clock: uptime is
    /// measured from here.
    pub started_at: jiff::Timestamp,
}

impl ForwardInfo {
    /// The user's name for it, else `pod/web 8080:80` or `service/api 0:http`: how the forward is
    /// named to the user.
    pub fn label(&self) -> String {
        label(&self.spec, self.local_addr)
    }
}

/// The name the user gave, else `kind/name local:remote`.
pub(crate) fn label(spec: &ForwardSpec, local_addr: Option<SocketAddr>) -> String {
    if let Some(name) = spec.name.as_deref().filter(|n| !n.is_empty()) {
        return name.to_owned();
    }
    let local = local_addr.map_or(spec.local_port, |a| a.port());
    format!(
        "{}/{} {local}:{}",
        spec.target.gvk.kind.to_ascii_lowercase(),
        spec.target.name,
        spec.remote_port
    )
}

/// What the manager follows: the `portforward.*` settings (E15-S08), global or for one cluster
/// (see [`PortForwardManager::config_for`](super::PortForwardManager::config_for)). A change
/// reaches forwards started afterwards; running ones are not restarted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForwardConfig {
    /// Whether forwards may start on a read-only cluster (`portforward.allowed_in_readonly`,
    /// default `false`).
    pub allowed_in_readonly: bool,
    /// Where a forward listens unless its request names an address (`portforward.default_bind`,
    /// default loopback).
    pub default_bind: IpAddr,
    /// The local ports a forward that asks for a free one picks from (`portforward.port_range`);
    /// `None` lets the OS pick.
    pub port_range: Option<PortRange>,
    /// Whether the k9s port-forward pod annotations are read as start-dialog suggestions
    /// (`portforward.annotations`, default `false`; read only, nothing is written).
    pub annotations: bool,
    /// The delays between reconnect attempts.
    pub backoff: Backoff,
}

impl Default for ForwardConfig {
    fn default() -> Self {
        Self {
            allowed_in_readonly: false,
            default_bind: ForwardSpec::DEFAULT_BIND,
            port_range: None,
            annotations: false,
            backoff: Backoff::default(),
        }
    }
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

impl StartError {
    /// The [`StartError`] an [`OxiError`] from a start command carries, if it is one.
    pub fn of(error: &OxiError) -> Option<&StartError> {
        std::error::Error::source(error)?.downcast_ref::<StartError>()
    }
}

impl From<StartError> for OxiError {
    fn from(err: StartError) -> Self {
        let message = match err.suggestion() {
            Some(hint) => format!("{err}. {hint}"),
            None => err.to_string(),
        };
        // The typed error rides along as the source, so a dialog can offer the matching fix
        // without parsing the message (see `StartError::of`).
        let kind = match &err {
            StartError::ReadOnly(_) => ErrorKind::Forbidden,
            StartError::NotConnected(_) | StartError::Unknown(_) => ErrorKind::NotFound,
            StartError::Unsupported(_) | StartError::Failed(_) => ErrorKind::Validation,
            StartError::Forward(inner) => OxiError::from(inner.clone()).kind(),
        };
        OxiError::new(kind, message).with_source(err)
    }
}
