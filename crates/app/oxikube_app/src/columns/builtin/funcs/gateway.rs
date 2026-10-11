//! Gateway API cells (GatewayClass, Gateway, HTTPRoute, GRPCRoute, TLSRoute), from the domain
//! view-models. The kinds are CRDs read as raw JSON, so no served version is assumed.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::view::ConditionStatus;
use oxikube_domain::view::network::{GatewayClassSummary, GatewaySummary, RouteSummary};

use crate::columns::{Cell, Tone};

/// A condition as a cell: `True` green, `False` red, `Unknown` (nothing reported yet) amber.
fn condition<'a>(status: ConditionStatus) -> Cell<'a> {
    let tone = match status {
        ConditionStatus::True => Tone::Ok,
        ConditionStatus::False => Tone::Error,
        ConditionStatus::Unknown => Tone::Warn,
    };
    Cell::text(status.as_str()).with_tone(tone)
}

/// `ACCEPTED` of a GatewayClass: whether its controller accepted it.
pub(crate) fn gateway_class_accepted<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    GatewayClassSummary::from_resource(res)
        .map_or_else(|_| Cell::empty(), |class| condition(class.accepted()))
}

/// `ADDRESSES` of a Gateway: where it is reachable.
pub(crate) fn gateway_addresses<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    gateway(res).map_or_else(Cell::empty, |g| Cell::text(g.addresses_text()))
}

/// `LISTENERS` of a Gateway: protocol, port and hostname of each.
pub(crate) fn gateway_listeners<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    gateway(res).map_or_else(Cell::empty, |g| Cell::text(g.listeners_text()))
}

/// `PROGRAMMED` of a Gateway: whether its data plane is configured.
pub(crate) fn gateway_programmed<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    gateway(res).map_or_else(Cell::empty, |g| condition(g.programmed()))
}

fn gateway(res: &Resource) -> Option<GatewaySummary> {
    GatewaySummary::from_resource(res).ok()
}

/// `HOSTNAMES` of a route (`*` for a route that names none).
pub(crate) fn route_hostnames<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    route(res).map_or_else(Cell::empty, |r| Cell::text(r.hostnames_text()))
}

/// `PARENTS` of a route: the Gateways it attaches to.
pub(crate) fn route_parents<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    route(res).map_or_else(Cell::empty, |r| Cell::text(r.parents_text()))
}

/// `RULES` of a route: how many.
pub(crate) fn route_rules<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    route(res).map_or_else(Cell::empty, |r| Cell::int(r.rules.len() as i64))
}

/// `ACCEPTED` of a route: whether every parent accepted it.
pub(crate) fn route_accepted<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    route(res).map_or_else(Cell::empty, |r| condition(r.accepted()))
}

fn route(res: &Resource) -> Option<RouteSummary> {
    RouteSummary::from_resource(res).ok()
}
