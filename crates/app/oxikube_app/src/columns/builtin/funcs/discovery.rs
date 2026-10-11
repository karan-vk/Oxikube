//! Service and networking cells: Service, Endpoints, EndpointSlice, Ingress, NetworkPolicy.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::json::JsonRef;
use oxikube_domain::view::{ServiceStatus, ServiceSummary};

use super::{arr_at, join_capped, selector_text, spec, status, str_at};
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

/// `status.loadBalancer.ingress[]` addresses (`ip`, else `hostname`), comma-separated.
fn load_balancer(res: &Resource) -> String {
    let lb = status(res).get("loadBalancer").unwrap_or(JsonRef::NULL);
    let addresses = arr_at(lb, "ingress")
        .iter()
        .filter_map(|i| str_at(i, "ip").or_else(|| str_at(i, "hostname")))
        .map(str::to_owned);
    join_capped(addresses, usize::MAX)
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

/// `CLASS` of an Ingress: `spec.ingressClassName`, else the legacy annotation.
pub(crate) fn ingress_class<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    str_at(spec(res), "ingressClassName")
        .or_else(|| {
            res.meta
                .annotations
                .get("kubernetes.io/ingress.class")
                .map(|v| &**v)
        })
        .map_or_else(Cell::empty, Cell::text)
}

/// `HOSTS` of an Ingress: each rule's host, `*` for a rule without one.
pub(crate) fn ingress_hosts<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let hosts = arr_at(spec(res), "rules")
        .iter()
        .map(|r| str_at(r, "host").unwrap_or("*").to_owned());
    Cell::text(join_capped(hosts, usize::MAX))
}

/// `ADDRESS` of an Ingress: its load balancer addresses.
pub(crate) fn ingress_address<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(load_balancer(res))
}

/// `PORTS` of an Ingress: `80`, plus `443` when it terminates TLS (kubectl).
pub(crate) fn ingress_ports<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let tls = !arr_at(spec(res), "tls").is_empty();
    Cell::text(if tls { "80, 443" } else { "80" })
}

/// `POD-SELECTOR` of a NetworkPolicy.
pub(crate) fn netpol_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(selector_text(
        res.json().pointer("/spec/podSelector/matchLabels"),
    ))
}

/// `POLICY TYPES` of a NetworkPolicy: `spec.policyTypes`, else `Ingress` (the API default).
pub(crate) fn netpol_types<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let types: Vec<&str> = arr_at(spec(res), "policyTypes")
        .iter()
        .filter_map(JsonRef::as_str)
        .collect();
    if types.is_empty() {
        Cell::text("Ingress")
    } else {
        Cell::text(types.join(","))
    }
}
