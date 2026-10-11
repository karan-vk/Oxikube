//! One ingress or egress rule of a NetworkPolicy: its peers and its ports, and how each reads.

use std::fmt;

use super::super::{arr_of, str_of};
use super::selector::selector_text;
use crate::json::JsonRef;

/// A label selector as a policy uses it: empty selects everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// `{}`: every pod, or every namespace.
    All,
    /// A selector that names labels, as [`selector_text`] prints it.
    Labels(String),
}

impl Selection {
    /// The selection `selector` makes. A selector that is present but empty is [`All`](Self::All).
    pub(super) fn of(selector: JsonRef<'_>) -> Self {
        selector_text(selector).map_or(Selection::All, Selection::Labels)
    }
}

/// One entry of a rule's `from` or `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyPeer {
    /// A `podSelector`, a `namespaceSelector`, or both. With only a pod selector the pods are
    /// those of the policy's own namespace; with only a namespace selector, every pod of the
    /// matching namespaces.
    Workloads {
        /// `podSelector`, when the entry has one.
        pods: Option<Selection>,
        /// `namespaceSelector`, when the entry has one.
        namespaces: Option<Selection>,
    },
    /// An `ipBlock`.
    IpBlock {
        /// The CIDR the block allows.
        cidr: String,
        /// CIDRs inside it that are not allowed.
        except: Vec<String>,
    },
    /// An entry none of the above reads (a field a newer API added): shown as such, never
    /// mistaken for "every peer".
    Unrecognised,
}

impl PolicyPeer {
    fn from_json(peer: JsonRef<'_>) -> Self {
        if let Some(block) = peer.get("ipBlock").filter(|b| b.is_object()) {
            return match str_of(block, "cidr") {
                Some(cidr) => PolicyPeer::IpBlock {
                    cidr: cidr.to_owned(),
                    except: arr_of(block, "except")
                        .iter()
                        .filter_map(JsonRef::as_str)
                        .map(str::to_owned)
                        .collect(),
                },
                None => PolicyPeer::Unrecognised,
            };
        }
        // A `null` selector is an absent one to the API server.
        let selector = |key: &str| peer.get(key).filter(|v| !v.is_null()).map(Selection::of);
        let pods = selector("podSelector");
        let namespaces = selector("namespaceSelector");
        if pods.is_some() || namespaces.is_some() {
            PolicyPeer::Workloads { pods, namespaces }
        } else {
            PolicyPeer::Unrecognised
        }
    }
}

impl fmt::Display for PolicyPeer {
    /// `pods app=web in this namespace`, `all pods in namespaces env=prod`,
    /// `IP 10.0.0.0/8 except 10.1.0.0/16`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyPeer::Unrecognised => f.write_str("unrecognised peer"),
            PolicyPeer::IpBlock { cidr, except } if except.is_empty() => write!(f, "IP {cidr}"),
            PolicyPeer::IpBlock { cidr, except } => {
                write!(f, "IP {cidr} except {}", except.join(", "))
            }
            PolicyPeer::Workloads { pods, namespaces } => {
                match pods {
                    Some(Selection::Labels(labels)) => write!(f, "pods {labels}")?,
                    Some(Selection::All) | None => f.write_str("all pods")?,
                }
                match namespaces {
                    None => f.write_str(" in this namespace"),
                    Some(Selection::All) => f.write_str(" in all namespaces"),
                    Some(Selection::Labels(labels)) => write!(f, " in namespaces {labels}"),
                }
            }
        }
    }
}

/// One entry of a rule's `ports`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyPort {
    /// `TCP` (the default), `UDP` or `SCTP`.
    pub protocol: String,
    /// A port number or the name of a container port; `None` is every port of the protocol.
    pub port: Option<String>,
    /// The last port of a range that starts at `port`.
    pub end_port: Option<u16>,
}

impl PolicyPort {
    fn from_json(port: JsonRef<'_>) -> Self {
        let value = port.get("port");
        Self {
            protocol: str_of(port, "protocol").unwrap_or("TCP").to_owned(),
            port: value
                .and_then(JsonRef::as_u64)
                .map(|n| n.to_string())
                .or_else(|| value.and_then(JsonRef::as_str).map(str::to_owned)),
            end_port: port
                .get("endPort")
                .and_then(JsonRef::as_u64)
                .and_then(|n| u16::try_from(n).ok()),
        }
    }
}

impl fmt::Display for PolicyPort {
    /// `TCP 80`, `TCP 8000-9000`, `TCP http`, `UDP all ports`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.port, self.end_port) {
            (None, _) => write!(f, "{} all ports", self.protocol),
            (Some(port), Some(end)) => write!(f, "{} {port}-{end}", self.protocol),
            (Some(port), None) => write!(f, "{} {port}", self.protocol),
        }
    }
}

/// One rule of `spec.ingress` or `spec.egress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRule {
    /// `from` or `to`. Empty (the field absent or `[]`) matches every peer.
    pub peers: Vec<PolicyPeer>,
    /// `ports`. Empty matches every port.
    pub ports: Vec<PolicyPort>,
}

impl PolicyRule {
    pub(super) fn from_json(rule: JsonRef<'_>, peers_key: &str) -> Self {
        Self {
            peers: arr_of(rule, peers_key)
                .iter()
                .map(PolicyPeer::from_json)
                .collect(),
            ports: arr_of(rule, "ports")
                .iter()
                .map(PolicyPort::from_json)
                .collect(),
        }
    }

    /// Whether the rule matches every peer on every port.
    pub fn is_allow_all(&self) -> bool {
        self.peers.is_empty() && self.ports.is_empty()
    }

    /// The peers as one line, `anywhere` when the rule names none.
    pub fn peers_text(&self) -> String {
        if self.peers.is_empty() {
            return "anywhere".to_owned();
        }
        self.peers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The ports as one line, `all ports` when the rule names none.
    pub fn ports_text(&self) -> String {
        if self.ports.is_empty() {
            return "all ports".to_owned();
        }
        self.ports
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The rules of `spec[key]`, each read with its peers under `peers_key`.
pub(super) fn rules_of(spec: JsonRef<'_>, key: &str, peers_key: &str) -> Vec<PolicyRule> {
    arr_of(spec, key)
        .iter()
        .map(|rule| PolicyRule::from_json(rule, peers_key))
        .collect()
}
