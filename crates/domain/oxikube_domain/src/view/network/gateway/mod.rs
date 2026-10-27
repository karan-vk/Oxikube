//! The Gateway API kinds, read from raw JSON (they are CRDs and may be served at `v1`, `v1beta1`
//! or `v1alpha2`).
//!
//! | File | Holds |
//! |---|---|
//! | `class` | [`GatewayClassSummary`]: controller, description, parameters and its `Accepted` condition |
//! | `gateway_summary` | [`GatewaySummary`]: class, listeners ([`ListenerSummary`]) with their attached routes and status, addresses ([`GatewayAddress`]) |
//! | `route` | [`RouteSummary`]: hostnames, parents ([`RouteParent`]) with the status each controller reported, rules |
//! | `rules` | [`RouteRule`] and [`RouteBackend`]: the matches, filters and backends of a route's rules |
//!
//! Gateway API status is where users debug, so every summary carries its conditions with the
//! reason and message the controller gave (`Accepted`, `Programmed`, `ResolvedRefs`).

mod class;
mod gateway_summary;
mod route;
mod rules;

use std::sync::Arc;

pub use class::GatewayClassSummary;
pub use gateway_summary::{GatewayAddress, GatewaySummary, ListenerSummary};
pub use route::{RouteKind, RouteParent, RouteSummary};
pub use rules::{RouteBackend, RouteRule};

use super::super::{Condition, ConditionStatus, arr_of, str_of};
use crate::json::JsonRef;

/// The Gateway API's group.
pub const GATEWAY_API_GROUP: &str = "gateway.networking.k8s.io";

/// Whether `group`/`kind` is a Gateway API kind the Network section lists itself (GatewayClass,
/// Gateway, HTTPRoute, GRPCRoute, TLSRoute), and so the Custom Resources section leaves out. The
/// group's other kinds (ReferenceGrant, BackendTLSPolicy, ...) stay with the custom resources.
pub fn is_gateway_network_kind(group: &str, kind: &str) -> bool {
    group == GATEWAY_API_GROUP
        && matches!(
            kind,
            "GatewayClass" | "Gateway" | "HTTPRoute" | "GRPCRoute" | "TLSRoute"
        )
}

/// A reference to another object, as the Gateway API spells them, with the defaults applied.
/// `version` is not part of it: the view that follows the reference asks discovery which
/// version the cluster serves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    /// API group; empty for the core group.
    pub group: Arc<str>,
    /// Kind, for example `Gateway` or `Service`.
    pub kind: Arc<str>,
    /// Namespace; `None` for a cluster-scoped kind.
    pub namespace: Option<Arc<str>>,
    /// Name.
    pub name: Arc<str>,
}

impl ObjectRef {
    /// The object `group`/`kind` `name`, in `namespace` for a namespaced kind.
    pub fn new(group: &str, kind: &str, namespace: Option<&str>, name: &str) -> Self {
        Self {
            group: group.into(),
            kind: kind.into(),
            namespace: namespace.map(Arc::from),
            name: name.into(),
        }
    }

    /// `namespace/name`, or the name alone for a cluster-scoped object.
    pub fn label(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{ns}/{}", self.name),
            None => self.name.to_string(),
        }
    }
}

/// The conditions of a `status` object or of one `status` entry, in order.
pub(super) fn conditions_of(v: JsonRef<'_>) -> Vec<Condition> {
    arr_of(v, "conditions")
        .iter()
        .filter_map(Condition::from_json)
        .collect()
}

/// The status of the condition of type `kind`; `Unknown` when the controller has not set it.
pub(super) fn status_of(conditions: &[Condition], kind: &str) -> ConditionStatus {
    conditions
        .iter()
        .find(|c| &*c.kind == kind)
        .map_or(ConditionStatus::Unknown, |c| c.status)
}

/// `v[key]` as a port number.
pub(super) fn port_of(v: JsonRef<'_>, key: &str) -> Option<u16> {
    v.get(key)?.as_u64().and_then(|n| u16::try_from(n).ok())
}

/// A `group` field: empty for the core group, `default` when the field is absent.
pub(super) fn group_of<'a>(v: JsonRef<'a>, default: &'a str) -> &'a str {
    match v.get("group").and_then(JsonRef::as_str) {
        Some(group) => group,
        None => default,
    }
}

/// `v[key]` as an owned string.
pub(super) fn string_of(v: JsonRef<'_>, key: &str) -> Option<String> {
    str_of(v, key).map(str::to_owned)
}
