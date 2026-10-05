//! Port-forward vocabulary: [`ForwardSpec`] says what to forward, [`ForwardStatus`] says how
//! a running forward is doing.
//!
//! A *forward* exposes one port of a pod, or of the pods behind a service, on a local TCP
//! listener. The adapter (`oxikube_kube::remote::portforward`) runs it; `PortForwardManager`
//! (E15) owns many of them and persists the specs as favourites, so [`ForwardSpec`] is
//! `serde`-stable.
//!
//! Binding defaults to loopback ([`ForwardSpec::DEFAULT_BIND`]): a forward exposes a cluster
//! endpoint to whoever can reach the listener.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::error::ErrorKind;
use crate::ids::{Gvk, ResourceRef};

/// A remote port: a number, or the name a container port or service port was given.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForwardPort {
    /// A port number.
    Number(u16),
    /// A port name (`http`). For a pod it is looked up in the containers' ports; for a
    /// service in the service's ports.
    Named(String),
}

impl std::fmt::Display for ForwardPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(port) => write!(f, "{port}"),
            Self::Named(name) => f.write_str(name),
        }
    }
}

/// What to forward and where to listen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForwardSpec {
    /// The target: a core `v1` `Pod` or `Service` (namespaced).
    pub target: ResourceRef,
    /// The port on the target. For a service this is the *service* port; the pod port it
    /// maps to (`targetPort`, possibly named) is resolved per pod.
    pub remote_port: ForwardPort,
    /// Local TCP port. `0` asks the OS for a free one; read it back from the running forward.
    #[serde(default)]
    pub local_port: u16,
    /// Local address to bind. Defaults to [`ForwardSpec::DEFAULT_BIND`] (loopback).
    #[serde(default = "default_bind")]
    pub bind: IpAddr,
}

fn default_bind() -> IpAddr {
    ForwardSpec::DEFAULT_BIND
}

impl ForwardSpec {
    /// Loopback only; see the module docs.
    pub const DEFAULT_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    /// A forward of `target`'s `remote_port` on a free loopback port.
    pub fn new(target: ResourceRef, remote_port: ForwardPort) -> Self {
        Self {
            target,
            remote_port,
            local_port: 0,
            bind: Self::DEFAULT_BIND,
        }
    }

    /// Listens on `local_port` instead of a free one.
    #[must_use]
    pub fn with_local_port(mut self, local_port: u16) -> Self {
        self.local_port = local_port;
        self
    }

    /// Binds `bind` instead of loopback.
    #[must_use]
    pub fn with_bind(mut self, bind: IpAddr) -> Self {
        self.bind = bind;
        self
    }

    /// Whether the target is a core `v1` `Service` (otherwise a `Pod`).
    pub fn targets_service(&self) -> bool {
        self.target.gvk == Gvk::new("", "v1", "Service")
    }
}

/// How a running forward is doing. Transitions are published in order; the last one before
/// the forward ends is always [`Stopped`](Self::Stopped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardStatus {
    /// Resolving the target and binding the listener.
    Starting,
    /// Accepting connections on `local_addr` and bridging them to `pod`.
    Listening {
        /// The bound local address (with the real port when `local_port` was `0`).
        local_addr: SocketAddr,
        /// The pod currently serving the forward.
        pod: String,
    },
    /// The pod behind the forward was deleted, is terminating, or stopped being usable.
    ///
    /// A service forward keeps listening and moves to another pod when one is available
    /// (new connections are refused until then); a pod forward ends with
    /// [`Stopped`](Self::Stopped) and leaves the decision to the manager.
    TargetGone {
        /// The pod that went away.
        pod: String,
    },
    /// A connection failed, or the forward could not continue. The forward is still running
    /// unless [`Stopped`](Self::Stopped) follows; the next connection that succeeds restores
    /// [`Listening`](Self::Listening).
    Error {
        /// What went wrong, in the port error taxonomy.
        kind: ErrorKind,
        /// A redacted one-line description for the UI.
        message: String,
    },
    /// The listener is closed and every bridged connection is torn down.
    Stopped,
}

impl ForwardStatus {
    /// Whether the forward is over: nothing more will be published after this.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClusterId, ContextName};

    fn target(kind: &str) -> ResourceRef {
        ResourceRef::new(
            ClusterId::new("kubeconfig", &ContextName::from("ctx")),
            Gvk::new("", "v1", kind),
            Some("default".into()),
            "web",
        )
    }

    #[test]
    fn defaults_listen_on_loopback_with_a_free_port() {
        let spec = ForwardSpec::new(target("Pod"), ForwardPort::Number(80));
        assert_eq!(spec.local_port, 0);
        assert!(spec.bind.is_loopback());
        assert!(!spec.targets_service());
        assert!(ForwardSpec::new(target("Service"), ForwardPort::Number(80)).targets_service());
    }

    #[test]
    fn spec_round_trips_and_fills_defaults() {
        let spec = ForwardSpec::new(target("Service"), ForwardPort::Named("http".into()))
            .with_local_port(8080);
        let json = serde_json::to_value(&spec).expect("serialise");
        let back: ForwardSpec = serde_json::from_value(json.clone()).expect("deserialise");
        assert_eq!(back, spec);

        let mut sparse = json;
        let object = sparse.as_object_mut().expect("object");
        object.remove("local_port");
        object.remove("bind");
        let back: ForwardSpec = serde_json::from_value(sparse).expect("defaults");
        assert_eq!(back.local_port, 0);
        assert_eq!(back.bind, ForwardSpec::DEFAULT_BIND);
    }

    #[test]
    fn only_stopped_is_terminal() {
        assert!(ForwardStatus::Stopped.is_terminal());
        assert!(!ForwardStatus::Starting.is_terminal());
        assert!(!ForwardStatus::TargetGone { pod: "p".into() }.is_terminal());
    }

    #[test]
    fn ports_display_as_written() {
        assert_eq!(ForwardPort::Number(8080).to_string(), "8080");
        assert_eq!(ForwardPort::Named("http".into()).to_string(), "http");
    }
}
