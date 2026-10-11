//! View-models of the Network section's kinds (E12-S14), built from raw JSON: Ingress,
//! IngressClass, NetworkPolicy and the Gateway API (GatewayClass, Gateway, HTTPRoute, GRPCRoute,
//! TLSRoute).
//!
//! The Gateway API kinds are CRDs, so there is no typed model and the served version is not
//! assumed: everything is read by key from the object's JSON, which is the same for `v1`,
//! `v1beta1` and `v1alpha2`. A missing or wrongly typed field degrades to a default; the only
//! error is passing the wrong kind ([`ViewError::WrongKind`](super::ViewError::WrongKind)).
//!
//! | File | Holds |
//! |---|---|
//! | `selector` | [`selector_text`]: a label selector as `k=v, key in (a,b)` text |
//! | `ingress` | [`IngressSummary`]: the rules flattened to one row per host and path, the TLS secret of each host, the load balancer addresses |
//! | `ingress_class` | [`IngressClassSummary`]: controller, parameters, the default flag ([`DEFAULT_CLASS_ANNOTATION`]) |
//! | `policy`, `policy_rule` | [`NetworkPolicySummary`]: policy types with the API's defaulting, the pod selector, and each ingress / egress rule as peers and ports ([`PolicyRule`], [`PolicyPeer`], [`PolicyPort`]) |
//! | `gateway` | the Gateway API: [`GatewayClassSummary`], [`GatewaySummary`] with its listeners, [`RouteSummary`] with hostnames, rules and parents, all with their conditions and reasons, and [`ObjectRef`], the way to the next object |
//!
//! # Provenance
//!
//! The defaults (`policyTypes`, an empty selector selects everything, `pathType`, the Gateway API
//! parent and backend references defaulting to the route's namespace) follow the Kubernetes and
//! Gateway API specifications; no code is copied.
//!
//! # Performance
//!
//! Constructors run once per object when a panel opens or the object changes, and for the
//! computed table cells once per visible row per frame behind the table's cell cache. They walk
//! the JSON by key and allocate only for the output strings.

mod gateway;
mod ingress;
mod ingress_class;
mod policy;
mod policy_rule;
mod selector;

pub use gateway::{
    GATEWAY_API_GROUP, GatewayAddress, GatewayClassSummary, GatewaySummary, ListenerSummary,
    ObjectRef, RouteBackend, RouteKind, RouteParent, RouteRule, RouteSummary,
    is_gateway_network_kind,
};
pub use ingress::{BackendPort, IngressBackend, IngressRuleRow, IngressSummary, IngressTls};
pub use ingress_class::{
    DEFAULT_CLASS_ANNOTATION, IngressClassParameters, IngressClassSummary, is_default_class,
};
pub use policy::{NetworkPolicySummary, PolicyEffect, PolicyType};
pub use policy_rule::{PolicyPeer, PolicyPort, PolicyRule, Selection};
pub use selector::selector_text;
