//! [`RouteSummary`]: an HTTPRoute, GRPCRoute or TLSRoute.

use super::super::super::{Condition, ConditionStatus, ViewError, arr_of, check_kind, str_of, sub};
use super::rules::RouteRule;
use super::{GATEWAY_API_GROUP, ObjectRef, conditions_of, group_of, port_of, status_of};
use crate::json::JsonRef;
use crate::resource::Resource;

/// Which route kind a [`RouteSummary`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteKind {
    /// `HTTPRoute`.
    Http,
    /// `GRPCRoute`.
    Grpc,
    /// `TLSRoute`.
    Tls,
}

/// A parent a route attaches to (a Gateway, normally) and what the controller said about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteParent {
    /// The parent; the group defaults to the Gateway API's, the kind to `Gateway` and the
    /// namespace to the route's.
    pub target: ObjectRef,
    /// `sectionName`: the listener of the Gateway the route attaches to.
    pub section: Option<String>,
    /// `port`: the Gateway port the route attaches to.
    pub port: Option<u16>,
    /// The controller that reported the status below (`status.parents[].controllerName`).
    pub controller: Option<String>,
    /// Its conditions: `Accepted`, `ResolvedRefs`, `PartiallyInvalid`, with their reasons. Empty
    /// until a controller has looked at the route.
    pub conditions: Vec<Condition>,
}

impl RouteParent {
    /// The status of the `Accepted` condition: whether the parent took the route.
    pub fn accepted(&self) -> ConditionStatus {
        status_of(&self.conditions, "Accepted")
    }

    /// `ns/gateway`, then `:listener` and `:port` when the reference narrows to one.
    pub fn label(&self) -> String {
        let mut text = self.target.label();
        if let Some(section) = &self.section {
            text.push_str(&format!(" · {section}"));
        }
        if let Some(port) = self.port {
            text.push_str(&format!(" :{port}"));
        }
        text
    }

    fn matches(&self, other: &ParentKey) -> bool {
        self.target == other.target && self.section == other.section && self.port == other.port
    }
}

/// A parent reference reduced to what identifies it.
struct ParentKey {
    target: ObjectRef,
    section: Option<String>,
    port: Option<u16>,
}

impl ParentKey {
    /// The parent this reference names, with what its controller reported (nothing, for a
    /// reference no status entry has matched yet).
    fn into_parent(self, controller: Option<String>, conditions: Vec<Condition>) -> RouteParent {
        RouteParent {
            target: self.target,
            section: self.section,
            port: self.port,
            controller,
            conditions,
        }
    }

    fn from_json(parent: JsonRef<'_>, route_namespace: Option<&str>) -> Option<Self> {
        Some(Self {
            target: ObjectRef::new(
                group_of(parent, GATEWAY_API_GROUP),
                str_of(parent, "kind").unwrap_or("Gateway"),
                str_of(parent, "namespace").or(route_namespace),
                str_of(parent, "name")?,
            ),
            section: str_of(parent, "sectionName").map(str::to_owned),
            port: port_of(parent, "port"),
        })
    }
}

/// A route: hostnames, rules and parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSummary {
    /// Which route kind.
    pub kind: RouteKind,
    /// `spec.hostnames`; empty matches every host the parent listener allows.
    pub hostnames: Vec<String>,
    /// `spec.parentRefs`, each with the status its controller reported.
    pub parents: Vec<RouteParent>,
    /// `spec.rules`.
    pub rules: Vec<RouteRule>,
}

impl RouteSummary {
    /// The summary of an HTTPRoute, GRPCRoute or TLSRoute.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "HTTPRoute, GRPCRoute or TLSRoute",
            &[
                (GATEWAY_API_GROUP, "HTTPRoute"),
                (GATEWAY_API_GROUP, "GRPCRoute"),
                (GATEWAY_API_GROUP, "TLSRoute"),
            ],
        )?;
        let kind = match &*res.kind.kind {
            "HTTPRoute" => RouteKind::Http,
            "GRPCRoute" => RouteKind::Grpc,
            _ => RouteKind::Tls,
        };
        let namespace = res.namespace();
        let spec = sub(res.json(), "spec");
        let status_entries = arr_of(sub(res.json(), "status"), "parents");

        let mut parents: Vec<RouteParent> = arr_of(spec, "parentRefs")
            .iter()
            .filter_map(|p| ParentKey::from_json(p, namespace))
            .map(|key| key.into_parent(None, Vec::new()))
            .collect();
        // Each status entry names the reference it is about; a controller reports one per
        // reference it handled, so several controllers can report on one reference.
        for entry in status_entries {
            let Some(key) = entry
                .get("parentRef")
                .and_then(|p| ParentKey::from_json(p, namespace))
            else {
                continue;
            };
            let conditions = conditions_of(entry);
            let controller = str_of(entry, "controllerName").map(str::to_owned);
            match parents
                .iter_mut()
                .find(|p| p.controller.is_none() && p.matches(&key))
            {
                Some(parent) => {
                    parent.controller = controller;
                    parent.conditions = conditions;
                }
                None => {
                    // A second controller for the same reference, or a status for a reference
                    // the spec no longer has (stale): shown, so the problem is visible.
                    parents.push(key.into_parent(controller, conditions));
                }
            }
        }
        Ok(Self {
            kind,
            hostnames: arr_of(spec, "hostnames")
                .iter()
                .filter_map(JsonRef::as_str)
                .map(str::to_owned)
                .collect(),
            rules: arr_of(spec, "rules")
                .iter()
                .map(|rule| RouteRule::from_json(rule, namespace))
                .collect(),
            parents,
        })
    }

    /// The `Accepted` state over all parents, whatever their order: `False` when any refused the
    /// route, else `Unknown` when any parent (or none at all) has not been heard from, else
    /// `True`.
    pub fn accepted(&self) -> ConditionStatus {
        let mut unknown = self.parents.is_empty();
        for parent in &self.parents {
            match parent.accepted() {
                ConditionStatus::False => return ConditionStatus::False,
                ConditionStatus::True => {}
                ConditionStatus::Unknown => unknown = true,
            }
        }
        if unknown {
            ConditionStatus::Unknown
        } else {
            ConditionStatus::True
        }
    }

    /// The hostnames as one line, comma-separated; `*` when the route names none.
    pub fn hostnames_text(&self) -> String {
        if self.hostnames.is_empty() {
            "*".to_owned()
        } else {
            self.hostnames.join(",")
        }
    }

    /// The parents as one line, comma-separated.
    pub fn parents_text(&self) -> String {
        self.parents
            .iter()
            .map(RouteParent::label)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
