//! [`EndpointSliceSummary`] and [`EndpointsSummary`]: where a Service's traffic goes (E12-S13).
//!
//! EndpointSlices (`discovery.k8s.io/v1`) are the current API; the core `Endpoints` object is
//! deprecated but still served, and both views keep it. A slice endpoint is *ready* unless its
//! `conditions.ready` is `false` (an absent condition means ready, per the API docs).

use std::iter::Sum;
use std::ops::Add;
use std::sync::Arc;

use super::{ViewError, arc_of, arr_of, check_kind, sub};
use crate::json::JsonRef;
use crate::resource::Resource;

/// The label that ties an EndpointSlice to its Service.
pub const SERVICE_NAME_LABEL: &str = "kubernetes.io/service-name";

/// How many endpoints are ready and how many are not, across any number of slices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadyCounts {
    /// Endpoints that can receive traffic.
    pub ready: u32,
    /// Endpoints that cannot (starting, failing readiness, or terminating).
    pub not_ready: u32,
}

impl ReadyCounts {
    /// `ready` of `total` endpoints are ready.
    fn of(total: usize, ready: usize) -> Self {
        Self {
            ready: u32::try_from(ready).unwrap_or(u32::MAX),
            not_ready: u32::try_from(total - ready).unwrap_or(u32::MAX),
        }
    }

    /// Every endpoint counted.
    pub const fn total(self) -> u32 {
        self.ready + self.not_ready
    }

    /// The text of a health summary: `3 ready`, `2 ready, 1 not ready` or `no endpoints`.
    pub fn summary(self) -> String {
        match (self.ready, self.not_ready) {
            (0, 0) => "no endpoints".to_owned(),
            (ready, 0) => format!("{ready} ready"),
            (0, not_ready) => format!("{not_ready} not ready"),
            (ready, not_ready) => format!("{ready} ready, {not_ready} not ready"),
        }
    }
}

impl Add for ReadyCounts {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            ready: self.ready.saturating_add(other.ready),
            not_ready: self.not_ready.saturating_add(other.not_ready),
        }
    }
}

impl Sum for ReadyCounts {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// What an endpoint points at (`targetRef`), usually a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointTarget {
    /// The kind, `Pod` for nearly every endpoint.
    pub kind: Arc<str>,
    /// The object's name.
    pub name: Arc<str>,
    /// The object's namespace, when set.
    pub namespace: Option<Arc<str>>,
}

impl EndpointTarget {
    fn from_json(v: JsonRef<'_>) -> Option<Self> {
        Some(Self {
            kind: arc_of(v, "kind").unwrap_or_else(|| Arc::from("Pod")),
            name: arc_of(v, "name")?,
            namespace: arc_of(v, "namespace"),
        })
    }
}

/// One endpoint of a slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSummary {
    /// `addresses`.
    pub addresses: Vec<Arc<str>>,
    /// `conditions.ready` (absent counts as ready).
    pub ready: bool,
    /// `conditions.serving` (absent follows `ready`).
    pub serving: bool,
    /// `conditions.terminating`.
    pub terminating: bool,
    /// `nodeName`.
    pub node: Option<Arc<str>>,
    /// `targetRef`.
    pub target: Option<EndpointTarget>,
}

impl EndpointSummary {
    fn from_json(v: JsonRef<'_>) -> Self {
        let conditions = sub(v, "conditions");
        let flag = |key: &str| conditions.get(key).and_then(JsonRef::as_bool);
        let ready = flag("ready").unwrap_or(true);
        Self {
            addresses: arr_of(v, "addresses")
                .iter()
                .filter_map(JsonRef::as_str)
                .map(Arc::from)
                .collect(),
            ready,
            serving: flag("serving").unwrap_or(ready),
            terminating: flag("terminating").unwrap_or(false),
            node: arc_of(v, "nodeName"),
            target: v.get("targetRef").and_then(EndpointTarget::from_json),
        }
    }
}

/// One port of a slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointPort {
    /// `name`.
    pub name: Option<Arc<str>>,
    /// `port`.
    pub port: Option<u16>,
    /// `protocol` (`TCP` when absent).
    pub protocol: Arc<str>,
}

impl EndpointPort {
    fn from_json(v: JsonRef<'_>) -> Self {
        Self {
            name: arc_of(v, "name"),
            port: v
                .get("port")
                .and_then(JsonRef::as_i64)
                .and_then(|n| u16::try_from(n).ok()),
            protocol: arc_of(v, "protocol").unwrap_or_else(|| Arc::from("TCP")),
        }
    }

    /// `8080/TCP`, or the protocol alone for a slice port without a number.
    pub fn text(&self) -> String {
        match self.port {
            Some(port) => format!("{port}/{}", self.protocol),
            None => self.protocol.to_string(),
        }
    }
}

/// An EndpointSlice, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSliceSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// The Service it belongs to (`kubernetes.io/service-name`).
    pub service: Option<Arc<str>>,
    /// `addressType`: `IPv4`, `IPv6` or `FQDN`.
    pub address_type: Arc<str>,
    /// `ports`.
    pub ports: Vec<EndpointPort>,
    /// `endpoints`.
    pub endpoints: Vec<EndpointSummary>,
}

impl EndpointSliceSummary {
    /// Reads a `discovery.k8s.io/v1` EndpointSlice.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "EndpointSlice",
            &[("discovery.k8s.io", "EndpointSlice")],
        )?;
        let json = res.json();
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            service: res.meta.labels.get(SERVICE_NAME_LABEL).cloned(),
            address_type: arc_of(json, "addressType").unwrap_or_else(|| Arc::from("IPv4")),
            ports: arr_of(json, "ports")
                .iter()
                .map(EndpointPort::from_json)
                .collect(),
            endpoints: arr_of(json, "endpoints")
                .iter()
                .map(EndpointSummary::from_json)
                .collect(),
        })
    }

    /// Ready and not-ready endpoints of this slice.
    pub fn ready_counts(&self) -> ReadyCounts {
        ReadyCounts::of(
            self.endpoints.len(),
            self.endpoints.iter().filter(|e| e.ready).count(),
        )
    }

    /// The ports as the `PORTS` column prints them.
    pub fn ports_text(&self) -> String {
        self.ports
            .iter()
            .map(EndpointPort::text)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// The ready and not-ready endpoint counts across `slices`.
pub fn ready_counts<'a>(slices: impl IntoIterator<Item = &'a EndpointSliceSummary>) -> ReadyCounts {
    slices
        .into_iter()
        .map(EndpointSliceSummary::ready_counts)
        .sum()
}

/// One address of a core `Endpoints` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointsAddress {
    /// `ip`.
    pub ip: Arc<str>,
    /// Whether it came from `addresses` (ready) rather than `notReadyAddresses`.
    pub ready: bool,
    /// `nodeName`.
    pub node: Option<Arc<str>>,
    /// `targetRef`.
    pub target: Option<EndpointTarget>,
}

/// The core `Endpoints` object, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointsSummary {
    /// `metadata.name`: the Service's name.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// Every address of every subset.
    pub addresses: Vec<EndpointsAddress>,
    /// Every distinct port of every subset.
    pub ports: Vec<EndpointPort>,
}

impl EndpointsSummary {
    /// Reads a core `v1` Endpoints.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Endpoints", &[("", "Endpoints")])?;
        let mut addresses = Vec::new();
        let mut ports: Vec<EndpointPort> = Vec::new();
        for subset in arr_of(res.json(), "subsets") {
            for (key, ready) in [("addresses", true), ("notReadyAddresses", false)] {
                for addr in arr_of(subset, key) {
                    if let Some(ip) = arc_of(addr, "ip") {
                        addresses.push(EndpointsAddress {
                            ip,
                            ready,
                            node: arc_of(addr, "nodeName"),
                            target: addr.get("targetRef").and_then(EndpointTarget::from_json),
                        });
                    }
                }
            }
            for p in arr_of(subset, "ports") {
                let port = EndpointPort::from_json(p);
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            addresses,
            ports,
        })
    }

    /// Ready and not-ready addresses.
    pub fn ready_counts(&self) -> ReadyCounts {
        ReadyCounts::of(
            self.addresses.len(),
            self.addresses.iter().filter(|a| a.ready).count(),
        )
    }
}
