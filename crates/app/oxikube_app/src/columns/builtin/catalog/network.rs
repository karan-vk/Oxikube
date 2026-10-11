//! Services and networking kinds.
//!
//! Sources: Freelens Service / Endpoints / EndpointSlice / Ingress / IngressClass /
//! NetworkPolicy lists (A1); k9s `svc`, `ep`, `ing`, `np` (A2); kubectl printers. The Gateway API
//! columns follow `kubectl get` for the CRDs plus the status conditions users debug with.

use super::super::def::{
    AGE, ColumnDef, KindDef, LABELS, NAME, NAMESPACE,
    Src::{Func, Text},
};
use super::super::funcs as f;

const SERVICE: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("type", "Type", Text("/spec/type")),
    ColumnDef::new("cluster-ip", "Cluster IP", Text("/spec/clusterIP")),
    ColumnDef::new("external-ip", "External IP", Func(f::service_external_ip)),
    ColumnDef::new("ports", "Ports", Func(f::service_ports)),
    ColumnDef::new("selector", "Selector", Func(f::service_selector)),
    ColumnDef::new("status", "Status", Func(f::service_status)),
    AGE,
    LABELS,
];

const ENDPOINTS: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("endpoints", "Endpoints", Func(f::endpoints_list)),
    AGE,
];

const ENDPOINT_SLICE: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("address-type", "Address Type", Text("/addressType")),
    ColumnDef::new("ports", "Ports", Func(f::slice_ports)),
    ColumnDef::new("endpoints", "Endpoints", Func(f::slice_endpoints)),
    AGE,
];

const INGRESS: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("address", "Load Balancers", Func(f::ingress_address)),
    ColumnDef::new("rules", "Rules", Func(f::ingress_rules)),
    AGE,
    ColumnDef::new("class", "Class", Func(f::ingress_class)).wide(),
    ColumnDef::new("hosts", "Hosts", Func(f::ingress_hosts)).wide(),
    ColumnDef::new("ports", "Ports", Func(f::ingress_ports)).wide(),
    LABELS,
];

const INGRESS_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("default", "Default", Func(f::ingress_class_default)),
    ColumnDef::new("controller", "Controller", Text("/spec/controller")),
    ColumnDef::new(
        "parameters-api-group",
        "API Group",
        Func(f::ingress_class_api_group),
    ),
    ColumnDef::new("parameters-scope", "Scope", Func(f::ingress_class_scope)),
    ColumnDef::new("parameters-kind", "Kind", Func(f::ingress_class_kind)),
    AGE,
    ColumnDef::new(
        "parameters-name",
        "Parameters Name",
        Func(f::ingress_class_parameters),
    )
    .wide(),
];

const NETWORK_POLICY: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("policy-types", "Policy Types", Func(f::netpol_types)),
    ColumnDef::new("pod-selector", "Pod Selector", Func(f::netpol_selector)),
    AGE,
    ColumnDef::new("ingress", "Ingress", Func(f::netpol_ingress)).wide(),
    ColumnDef::new("egress", "Egress", Func(f::netpol_egress)).wide(),
];

// The Gateway API (E12-S14). Its kinds are CRDs, but small and read as raw JSON, so they use a
// full feed (see the feed policy) and these computed columns instead of the Table API's.

const GATEWAY_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("controller", "Controller", Text("/spec/controllerName")),
    ColumnDef::new("accepted", "Accepted", Func(f::gateway_class_accepted)),
    AGE,
    ColumnDef::new("description", "Description", Text("/spec/description")).wide(),
];

const GATEWAY: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("class", "Class", Text("/spec/gatewayClassName")),
    ColumnDef::new("addresses", "Addresses", Func(f::gateway_addresses)),
    ColumnDef::new("listeners", "Listeners", Func(f::gateway_listeners)),
    ColumnDef::new("programmed", "Programmed", Func(f::gateway_programmed)),
    AGE,
];

const ROUTE: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("hostnames", "Hostnames", Func(f::route_hostnames)),
    ColumnDef::new("parents", "Parents", Func(f::route_parents)),
    ColumnDef::new("rules", "Rules", Func(f::route_rules)).number(),
    ColumnDef::new("accepted", "Accepted", Func(f::route_accepted)),
    AGE,
];

pub(super) const KINDS: &[KindDef] = &[
    KindDef {
        group: "",
        kind: "Service",
        columns: SERVICE,
    },
    KindDef {
        group: "",
        kind: "Endpoints",
        columns: ENDPOINTS,
    },
    KindDef {
        group: "discovery.k8s.io",
        kind: "EndpointSlice",
        columns: ENDPOINT_SLICE,
    },
    KindDef {
        group: "networking.k8s.io",
        kind: "Ingress",
        columns: INGRESS,
    },
    KindDef {
        group: "networking.k8s.io",
        kind: "IngressClass",
        columns: INGRESS_CLASS,
    },
    KindDef {
        group: "networking.k8s.io",
        kind: "NetworkPolicy",
        columns: NETWORK_POLICY,
    },
    KindDef {
        group: "gateway.networking.k8s.io",
        kind: "GatewayClass",
        columns: GATEWAY_CLASS,
    },
    KindDef {
        group: "gateway.networking.k8s.io",
        kind: "Gateway",
        columns: GATEWAY,
    },
    KindDef {
        group: "gateway.networking.k8s.io",
        kind: "HTTPRoute",
        columns: ROUTE,
    },
    KindDef {
        group: "gateway.networking.k8s.io",
        kind: "GRPCRoute",
        columns: ROUTE,
    },
    KindDef {
        group: "gateway.networking.k8s.io",
        kind: "TLSRoute",
        columns: ROUTE,
    },
];
