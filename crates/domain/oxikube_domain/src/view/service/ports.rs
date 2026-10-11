//! [`ServicePortSummary`]: one entry of `spec.ports`, as kubectl prints it and as a browser link
//! needs it.

use std::sync::Arc;

use super::super::{arc_of, str_of};
use crate::json::JsonRef;

/// A `targetPort`: a number, or the name of a container port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetPort {
    /// A container port number.
    Number(u16),
    /// The name a container port was given; resolved against a pod's containers with
    /// [`resolve_target_port`](super::resolve_target_port).
    Named(Arc<str>),
}

impl std::fmt::Display for TargetPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Named(name) => f.write_str(name),
        }
    }
}

/// One port of a Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePortSummary {
    /// `name`; ports of a single-port Service may have none.
    pub name: Option<Arc<str>>,
    /// `port`: the number clients use on the Service.
    pub port: u16,
    /// `protocol`: `TCP` (the default), `UDP` or `SCTP`.
    pub protocol: Arc<str>,
    /// `targetPort`; the Service `port` when the field is absent.
    pub target_port: TargetPort,
    /// `nodePort`, when allocated.
    pub node_port: Option<u16>,
    /// `appProtocol` (`http`, `https`, `kubernetes.io/h2c`, ...).
    pub app_protocol: Option<Arc<str>>,
}

impl ServicePortSummary {
    /// Reads one `spec.ports` entry; `None` without a usable `port`.
    pub(super) fn from_json(v: JsonRef<'_>) -> Option<Self> {
        let port = u16::try_from(v.get("port")?.as_i64()?).ok()?;
        let target_port = match v.get("targetPort") {
            Some(t) if t.is_string() => t
                .as_str()
                .filter(|s| !s.is_empty())
                .map_or(TargetPort::Number(port), |s| {
                    TargetPort::Named(Arc::from(s))
                }),
            Some(t) => t
                .as_i64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| *n != 0)
                .map_or(TargetPort::Number(port), TargetPort::Number),
            None => TargetPort::Number(port),
        };
        Some(Self {
            name: arc_of(v, "name"),
            port,
            protocol: arc_of(v, "protocol").unwrap_or_else(|| Arc::from("TCP")),
            target_port,
            node_port: v
                .get("nodePort")
                .and_then(JsonRef::as_i64)
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| *n != 0),
            app_protocol: arc_of(v, "appProtocol"),
        })
    }

    /// kubectl's text for the `PORT(S)` column: `80/TCP`, or `80:30080/TCP` with a node port.
    pub fn kubectl_text(&self) -> String {
        match self.node_port {
            Some(node) => format!("{}:{node}/{}", self.port, self.protocol),
            None => format!("{}/{}", self.port, self.protocol),
        }
    }

    /// Whether the port speaks TCP, which is all a port-forward (and so a browser) can reach.
    pub fn is_tcp(&self) -> bool {
        self.protocol.eq_ignore_ascii_case("TCP")
    }

    /// The URL scheme to open this port with: `https` for an `https` application protocol, a
    /// port named `https` or the usual TLS ports, else `http`.
    pub fn browser_scheme(&self) -> &'static str {
        let secure_name =
            |s: &str| s.eq_ignore_ascii_case("https") || s.eq_ignore_ascii_case("tls");
        let named = self.app_protocol.as_deref().is_some_and(secure_name)
            || self.name.as_deref().is_some_and(secure_name);
        if named || matches!(self.port, 443 | 8443) {
            "https"
        } else {
            "http"
        }
    }

    /// The URL of a forward to this port that listens on `local_port` of this machine.
    pub fn browser_url(&self, local_port: u16) -> String {
        format!("{}://localhost:{local_port}", self.browser_scheme())
    }

    /// The label a link uses: the port's name when it has one, with its number.
    pub fn label(&self) -> String {
        match self.name.as_deref() {
            Some(name) => format!("{name} ({})", self.port),
            None => self.port.to_string(),
        }
    }
}

/// The port number a Service port reaches on a pod: `target` as it is when numeric, else the
/// `containerPort` of the container port of that name and protocol in the first of `pods` that
/// has one (init containers included, as sidecars are). `None` when no pod names it.
///
/// Pod specs are read from their JSON, so the pods may be `Resource`s of the store.
pub fn resolve_target_port<'a>(
    port: &ServicePortSummary,
    pods: impl IntoIterator<Item = &'a crate::resource::Resource>,
) -> Option<u16> {
    let name = match &port.target_port {
        TargetPort::Number(n) => return Some(*n),
        TargetPort::Named(name) => name,
    };
    pods.into_iter().find_map(|pod| {
        let spec = pod.json().get("spec")?;
        ["containers", "initContainers"]
            .into_iter()
            .find_map(|key| {
                spec.get(key)?.as_array()?.iter().find_map(|container| {
                    container
                        .get("ports")?
                        .as_array()?
                        .iter()
                        .filter(|p| str_of(*p, "name") == Some(&**name))
                        .filter(|p| {
                            str_of(*p, "protocol")
                                .unwrap_or("TCP")
                                .eq_ignore_ascii_case(&port.protocol)
                        })
                        .find_map(|p| u16::try_from(p.get("containerPort")?.as_i64()?).ok())
                })
            })
    })
}
