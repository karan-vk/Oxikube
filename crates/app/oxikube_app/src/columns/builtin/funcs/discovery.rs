//! Service and discovery cells: Service, Endpoints, EndpointSlice (Ingress, IngressClass and
//! NetworkPolicy are in `network`, the Gateway API in `gateway`).

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::json::JsonRef;
use oxikube_domain::view::{ServiceStatus, ServiceSummary};

use super::{arr_at, join_capped, selector_text, str_at};
use crate::columns::{Cell, Tone};

/// `PORT(S)` of a Service: `{port}/{protocol}` or `{port}:{nodePort}/{protocol}` per port,
/// comma-separated (the rule lives in [`ServiceSummary`]).
pub(crate) fn service_ports<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    service(res).map_or_else(Cell::empty, |svc| Cell::text(svc.ports_text()))
}

/// `EXTERNAL-IP` by kubectl's rules (the load balancer's addresses and `spec.externalIPs`,
/// `<pending>` while a load balancer has none, an ExternalName's name).
pub(crate) fn service_external_ip<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    service(res).map_or_else(Cell::empty, |svc| {
        let text = svc.external_ip_text();
        let tone = if text == "<pending>" {
            Tone::Warn
        } else {
            Tone::Neutral
        };
        Cell::text(text).with_tone(tone)
    })
}

/// `SELECTOR` of a Service: `spec.selector`.
pub(crate) fn service_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(selector_text(res.json().pointer("/spec/selector")))
}

/// `STATUS` of a Service: `Pending` / `Assigned` for a load balancer, `Active` for the rest.
pub(crate) fn service_status<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    service(res).map_or_else(Cell::empty, |svc| {
        let tone = match svc.status {
            ServiceStatus::Pending => Tone::Warn,
            ServiceStatus::Assigned | ServiceStatus::Active => Tone::Ok,
        };
        Cell::text(svc.status.as_str()).with_tone(tone)
    })
}

fn service(res: &Resource) -> Option<ServiceSummary> {
    ServiceSummary::from_resource(res).ok()
}

/// `ENDPOINTS` of an Endpoints object: `ip:port` pairs, at most three then `+ N more...`.
pub(crate) fn endpoints_list<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let mut pairs = Vec::new();
    for subset in arr_at(res.json(), "subsets") {
        let ports = arr_at(subset, "ports");
        for addr in arr_at(subset, "addresses") {
            let Some(ip) = str_at(addr, "ip") else {
                continue;
            };
            if ports.is_empty() {
                pairs.push(ip.to_owned());
            }
            for port in ports {
                if let Some(n) = port.get("port").and_then(JsonRef::as_i64) {
                    pairs.push(format!("{ip}:{n}"));
                }
            }
        }
    }
    Cell::text(join_capped(pairs, 3))
}

/// `PORTS` of an EndpointSlice: port numbers, comma-separated.
pub(crate) fn slice_ports<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let ports = arr_at(res.json(), "ports")
        .iter()
        .filter_map(|p| p.get("port").and_then(JsonRef::as_i64))
        .map(|n| n.to_string());
    Cell::text(join_capped(ports, usize::MAX))
}

/// `ENDPOINTS` of an EndpointSlice: the first address of each endpoint, at most three.
pub(crate) fn slice_endpoints<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let addrs = arr_at(res.json(), "endpoints")
        .iter()
        .filter_map(|e| arr_at(e, "addresses").first()?.as_str())
        .map(str::to_owned);
    Cell::text(join_capped(addrs, 3))
}
