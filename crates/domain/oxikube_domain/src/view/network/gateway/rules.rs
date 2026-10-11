//! The rules of a route: what they match, which filters they apply, where they send traffic.

use super::super::super::{arr_of, str_of, sub};
use super::{ObjectRef, group_of, port_of};
use crate::json::JsonRef;

/// A backend a rule sends traffic to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteBackend {
    /// The object, a Service unless `kind` says otherwise; the namespace defaults to the route's.
    pub target: ObjectRef,
    /// `port`.
    pub port: Option<u16>,
    /// `weight`; `None` is the default weight of 1.
    pub weight: Option<i64>,
}

impl RouteBackend {
    /// `web:8080`, with the weight when it is not the default: `web:8080 (weight 20)`.
    pub fn label(&self) -> String {
        let mut text = self.target.name.to_string();
        if &*self.target.kind != "Service" {
            text = format!("{}/{text}", self.target.kind);
        }
        if let Some(port) = self.port {
            text.push_str(&format!(":{port}"));
        }
        if let Some(weight) = self.weight {
            text.push_str(&format!(" (weight {weight})"));
        }
        text
    }

    fn from_json(backend: JsonRef<'_>, route_namespace: Option<&str>) -> Option<Self> {
        let namespace = str_of(backend, "namespace").or(route_namespace);
        Some(Self {
            target: ObjectRef::new(
                group_of(backend, ""),
                str_of(backend, "kind").unwrap_or("Service"),
                namespace,
                str_of(backend, "name")?,
            ),
            port: port_of(backend, "port"),
            weight: backend.get("weight").and_then(JsonRef::as_i64),
        })
    }
}

/// One rule of a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRule {
    /// What each match reads like (`PathPrefix /api & header x-env=canary`); empty for a route
    /// kind without matches (TLSRoute) and for an HTTPRoute rule that matches every request.
    pub matches: Vec<String>,
    /// The filters the rule applies, by type (`RequestHeaderModifier`, `URLRewrite`, ...).
    pub filters: Vec<String>,
    /// `backendRefs`.
    pub backends: Vec<RouteBackend>,
}

impl RouteRule {
    pub(super) fn from_json(rule: JsonRef<'_>, route_namespace: Option<&str>) -> Self {
        Self {
            matches: arr_of(rule, "matches").iter().map(match_text).collect(),
            filters: arr_of(rule, "filters")
                .iter()
                .filter_map(|f| str_of(f, "type"))
                .map(str::to_owned)
                .collect(),
            backends: arr_of(rule, "backendRefs")
                .iter()
                .filter_map(|b| RouteBackend::from_json(b, route_namespace))
                .collect(),
        }
    }
}

/// One match of an HTTPRoute or GRPCRoute rule, as text. Every condition of a match must hold,
/// so they are joined with `&`.
fn match_text(m: JsonRef<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    // HTTPRoute.
    if let Some(path) = m.get("path").filter(|p| p.is_object()) {
        let kind = str_of(path, "type").unwrap_or("PathPrefix");
        parts.push(format!("{kind} {}", str_of(path, "value").unwrap_or("/")));
    }
    if let Some(method) = str_of(m, "method").filter(|_| !sub(m, "method").is_object()) {
        parts.push(format!("method {method}"));
    }
    // GRPCRoute.
    if let Some(method) = m.get("method").filter(|v| v.is_object()) {
        let service = str_of(method, "service");
        let name = str_of(method, "method");
        parts.push(match (service, name) {
            (Some(service), Some(name)) => format!("gRPC {service}/{name}"),
            (Some(service), None) => format!("gRPC {service}/*"),
            (None, Some(name)) => format!("gRPC */{name}"),
            (None, None) => "gRPC any method".to_owned(),
        });
    }
    for header in arr_of(m, "headers") {
        parts.push(pair_text("header", header));
    }
    for query in arr_of(m, "queryParams") {
        parts.push(pair_text("query", query));
    }
    if parts.is_empty() {
        "any request".to_owned()
    } else {
        parts.join(" & ")
    }
}

/// `header x-env=canary`, or `header x-env~canary.*` for a regular expression match.
fn pair_text(what: &str, pair: JsonRef<'_>) -> String {
    let op = if str_of(pair, "type") == Some("RegularExpression") {
        "~"
    } else {
        "="
    };
    format!(
        "{what} {}{op}{}",
        str_of(pair, "name").unwrap_or("?"),
        str_of(pair, "value").unwrap_or_default()
    )
}
