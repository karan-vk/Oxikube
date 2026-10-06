//! Services and networking kinds.
//!
//! Sources: Freelens Service / Endpoints / EndpointSlice / Ingress / IngressClass /
//! NetworkPolicy lists (A1); k9s `svc`, `ep`, `ing`, `np` (A2); kubectl printers.

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
    AGE,
    ColumnDef::new("selector", "Selector", Func(f::service_selector)).wide(),
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
    ColumnDef::new("class", "Class", Func(f::ingress_class)),
    ColumnDef::new("hosts", "Hosts", Func(f::ingress_hosts)),
    ColumnDef::new("address", "Address", Func(f::ingress_address)),
    ColumnDef::new("ports", "Ports", Func(f::ingress_ports)),
    AGE,
    LABELS,
];

const INGRESS_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("controller", "Controller", Text("/spec/controller")),
    AGE,
    ColumnDef::new(
        "parameters-kind",
        "Parameters Kind",
        Text("/spec/parameters/kind"),
    )
    .wide(),
    ColumnDef::new(
        "parameters-name",
        "Parameters Name",
        Text("/spec/parameters/name"),
    )
    .wide(),
];

const NETWORK_POLICY: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("pod-selector", "Pod Selector", Func(f::netpol_selector)),
    ColumnDef::new("policy-types", "Policy Types", Func(f::netpol_types)),
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
];
