//! [`FavouriteForward`]: a port-forward saved to start again, and its stable [`FavouriteId`].

use oxikube_domain::ids::ClusterId;
use oxikube_domain::portforward::ForwardSpec;
use serde::{Deserialize, Serialize};

/// A favourite's id: derived from what it forwards (cluster, target, remote port, local port,
/// bind address), so saving the same forward twice is one favourite. Stable across runs, valid as
/// a state key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FavouriteId(String);

impl FavouriteId {
    /// The id of the favourite forwarding `spec`.
    pub fn of(spec: &ForwardSpec) -> Self {
        let t = &spec.target;
        // FNV-1a: stable across Rust versions and runs, unlike `DefaultHasher`.
        let identity = format!(
            "{}|{}/{}|{}|{}|{}|{}|{}",
            t.cluster,
            t.gvk.group,
            t.gvk.kind,
            t.namespace.as_deref().unwrap_or(""),
            t.name,
            spec.remote_port,
            spec.local_port,
            spec.bind
        );
        let hash = identity.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        Self(format!("fav-{hash:016x}"))
    }

    /// Wraps an id typed or stored by the user (checked against the store when used).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FavouriteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A port-forward saved to start again when its cluster connects.
///
/// Holds only a cluster id, a resource reference, ports, a bind address and a display name: no
/// kubeconfig detail, no credential, nothing from the cluster (ADR 0010, non-negotiable 5). A
/// local port of `0` in the spec is "Auto": a free port each time, never a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavouriteForward {
    /// The favourite's id.
    pub id: FavouriteId,
    /// What it forwards and where it listens.
    pub spec: ForwardSpec,
    /// What the user calls it (`staging db`).
    pub name: String,
    /// Whether it starts when the cluster connects.
    #[serde(default = "yes")]
    pub auto_start: bool,
    /// When it was saved, in milliseconds since the Unix epoch.
    #[serde(default)]
    pub created_ms: i64,
}

fn yes() -> bool {
    true
}

impl FavouriteForward {
    /// A favourite of `spec`, named `name` (or after the target and port), starting on connect.
    pub fn new(spec: ForwardSpec, name: Option<String>, created_ms: i64) -> Self {
        let name = name
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| default_name(&spec));
        Self {
            id: FavouriteId::of(&spec),
            spec,
            name,
            auto_start: true,
            created_ms,
        }
    }

    /// The cluster it belongs to.
    pub fn cluster(&self) -> &ClusterId {
        &self.spec.target.cluster
    }

    /// `pod/db 5432:5432`, or `service/api auto:http`: the forward as the notifications name it.
    pub fn label(&self) -> String {
        let local = match self.spec.local_port {
            0 => "auto".to_owned(),
            port => port.to_string(),
        };
        format!(
            "{}/{} {local}:{}",
            self.spec.target.gvk.kind.to_ascii_lowercase(),
            self.spec.target.name,
            self.spec.remote_port
        )
    }
}

/// `pod/db:5432`.
fn default_name(spec: &ForwardSpec) -> String {
    format!(
        "{}/{}:{}",
        spec.target.gvk.kind.to_ascii_lowercase(),
        spec.target.name,
        spec.remote_port
    )
}
