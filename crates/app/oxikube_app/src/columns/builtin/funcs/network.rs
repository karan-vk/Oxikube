//! Ingress, IngressClass and NetworkPolicy cells, from the domain view-models.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::view::network::{
    IngressClassParameters, IngressClassSummary, IngressSummary, NetworkPolicySummary,
    PolicyEffect, PolicyType, is_default_class,
};

use crate::columns::{Cell, Tone};

/// `CLASS` of an Ingress: `spec.ingressClassName`, else the legacy annotation.
pub(crate) fn ingress_class<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res)
        .and_then(|ingress| ingress.class)
        .map_or_else(Cell::empty, |class| Cell::text(class.to_string()))
}

/// `HOSTS` of an Ingress: each rule's host, `*` for a rule without one.
pub(crate) fn ingress_hosts<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res).map_or_else(Cell::empty, |ingress| Cell::text(ingress.hosts().join(",")))
}

/// `LOAD BALANCERS` of an Ingress: the addresses in `status.loadBalancer`.
pub(crate) fn ingress_address<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res).map_or_else(Cell::empty, |ingress| {
        Cell::text(ingress.load_balancers.join(","))
    })
}

/// `RULES` of an Ingress: `host/path → service:port` for every rule, the default backend last.
pub(crate) fn ingress_rules<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res).map_or_else(Cell::empty, |ingress| Cell::text(ingress.rules_text()))
}

/// `PORTS` of an Ingress: `80`, plus `443` when it terminates TLS (kubectl).
pub(crate) fn ingress_ports<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    summary(res).map_or_else(Cell::empty, |ingress| {
        Cell::text(if ingress.tls.is_empty() {
            "80"
        } else {
            "80, 443"
        })
    })
}

fn summary(res: &Resource) -> Option<IngressSummary> {
    IngressSummary::from_resource(res).ok()
}

/// `DEFAULT` of an IngressClass: `Default` for the class carrying the default annotation.
pub(crate) fn ingress_class_default<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    if is_default_class(res) {
        Cell::text("Default").with_tone(Tone::Ok)
    } else {
        Cell::empty()
    }
}

/// `API GROUP` of an IngressClass' parameters (blank for the core group or no parameters).
pub(crate) fn ingress_class_api_group<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    class_parameters(res, |p| p.api_group.clone())
}

/// `SCOPE` of an IngressClass' parameters: `Cluster` or `Namespace`.
pub(crate) fn ingress_class_scope<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    class_parameters(res, |p| Some(p.scope.clone()))
}

/// `KIND` of an IngressClass' parameters.
pub(crate) fn ingress_class_kind<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    class_parameters(res, |p| Some(p.kind.clone()))
}

/// `PARAMETERS NAME`, `namespace/name` for a namespaced parameters object.
pub(crate) fn ingress_class_parameters<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    class_parameters(res, |p| {
        Some(match &p.namespace {
            Some(ns) => format!("{ns}/{}", p.name),
            None => p.name.clone(),
        })
    })
}

fn class_parameters<'a>(
    res: &Resource,
    read: impl FnOnce(&IngressClassParameters) -> Option<String>,
) -> Cell<'a> {
    IngressClassSummary::from_resource(res)
        .ok()
        .and_then(|class| class.parameters)
        .and_then(|parameters| read(&parameters))
        .map_or_else(Cell::empty, Cell::text)
}

/// `POD SELECTOR` of a NetworkPolicy: its labels, `all pods` for the empty selector.
pub(crate) fn netpol_selector<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    policy(res).map_or_else(Cell::empty, |p| Cell::text(p.pod_selector_text()))
}

/// `POLICY TYPES` of a NetworkPolicy, with the API's defaulting.
pub(crate) fn netpol_types<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    policy(res).map_or_else(Cell::empty, |p| {
        let types: Vec<&str> = p.policy_types.iter().map(|t| t.as_str()).collect();
        Cell::text(types.join(","))
    })
}

/// `INGRESS` of a NetworkPolicy: what it does to ingress traffic.
pub(crate) fn netpol_ingress<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    effect_cell(res, PolicyType::Ingress)
}

/// `EGRESS` of a NetworkPolicy: what it does to egress traffic.
pub(crate) fn netpol_egress<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    effect_cell(res, PolicyType::Egress)
}

/// `Deny all` (default deny), `Allow all`, `N rules`, or blank when the policy does not cover
/// the direction.
fn effect_cell<'a>(res: &Resource, direction: PolicyType) -> Cell<'a> {
    let Some(policy) = policy(res) else {
        return Cell::empty();
    };
    let rules = match direction {
        PolicyType::Ingress => policy.ingress.len(),
        PolicyType::Egress => policy.egress.len(),
    };
    match policy.effect(direction) {
        PolicyEffect::NotCovered => Cell::empty(),
        PolicyEffect::DenyAll => Cell::text("Deny all").with_tone(Tone::Warn),
        PolicyEffect::AllowAll => Cell::text("Allow all"),
        PolicyEffect::Restricted => Cell::text(format!("{rules} rule{}", plural(rules))),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn policy(res: &Resource) -> Option<NetworkPolicySummary> {
    NetworkPolicySummary::from_resource(res).ok()
}
