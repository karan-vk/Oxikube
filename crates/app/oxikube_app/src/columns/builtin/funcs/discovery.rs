//! Service and discovery cells: Service, Endpoints, EndpointSlice (Ingress, IngressClass and
//! NetworkPolicy are in `network`, the Gateway API in `gateway`).

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::json::JsonRef;

use super::{arr_at, join_capped, selector_text, spec, status, str_at};
use crate::columns::Cell;

/// `{port}/{protocol}` or `{port}:{nodePort}/{protocol}` per service port, comma-separated.
pub(crate) fn service_ports<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let ports = arr_at(spec(res), "ports").iter().filter_map(|p| {
        let port = p.get("port")?.as_i64()?;
        let proto = str_at(p, "protocol").unwrap_or("TCP");
        Some(match p.get("nodePort").and_then(JsonRef::as_i64) {
            Some(node) if node != 0 => format!("{port}:{node}/{proto}"),
            _ => format!("{port}/{proto}"),
        })
    });
    Cell::text(join_capped(ports, usize::MAX))
}

/// `EXTERNAL-IP`: load balancer ingress, else `spec.externalIPs`, else the `ExternalName`.
pub(crate) fn service_external_ip<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let lb = load_balancer(res);
    if !lb.is_empty() {
        return Cell::text(lb);
    }
    let spec = spec(res);
    let external: Vec<&str> = arr_at(spec, "externalIPs")
        .iter()
        .filter_map(JsonRef::as_str)
        .collect();
    if !external.is_empty() {
        return Cell::text(external.join(","));
    }
    str_at(spec, "externalName").map_or_else(Cell::empty, Cell::text)
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

/// `SELECTOR` of a Service: `spec.selector`.
pub(crate) fn service_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(selector_text(res.json().pointer("/spec/selector")))
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
