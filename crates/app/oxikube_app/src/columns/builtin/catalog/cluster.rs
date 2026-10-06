//! Cluster-level kinds: Node, Namespace, Event, CustomResourceDefinition, scheduling and
//! admission kinds.
//!
//! Sources: Freelens Node / Namespace / Event / CustomResourceDefinition / PriorityClass /
//! RuntimeClass / webhook configuration / ValidatingAdmissionPolicy lists (A1); k9s `no`, `ns`,
//! `ev`, `crd` (A2); kubectl printers.

use super::super::def::{
    AGE, CPU, ColumnDef, KindDef, LABELS, MEMORY, NAME, NAMESPACE,
    Src::{Func, Int, Len, Qty, Text},
};
use super::super::funcs as f;

const NODE: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("status", "Status", Func(f::node_status)),
    ColumnDef::new("roles", "Roles", Func(f::node_roles)),
    ColumnDef::new("version", "Version", Func(f::node_version)),
    ColumnDef::new("internal-ip", "Internal IP", Func(f::node_internal_ip)),
    CPU,
    MEMORY,
    ColumnDef::new("taints", "Taints", Func(f::node_taints)).number(),
    AGE,
    ColumnDef::new("external-ip", "External IP", Func(f::node_external_ip)).wide(),
    ColumnDef::new("os-image", "OS Image", Text("/status/nodeInfo/osImage")).wide(),
    ColumnDef::new("kernel", "Kernel", Text("/status/nodeInfo/kernelVersion")).wide(),
    ColumnDef::new(
        "container-runtime",
        "Container Runtime",
        Text("/status/nodeInfo/containerRuntimeVersion"),
    )
    .wide(),
    ColumnDef::new("arch", "Arch", Text("/status/nodeInfo/architecture")).wide(),
    ColumnDef::new(
        "allocatable-cpu",
        "Allocatable CPU",
        Qty("/status/allocatable/cpu"),
    )
    .quantity()
    .wide(),
    ColumnDef::new(
        "allocatable-memory",
        "Allocatable Memory",
        Qty("/status/allocatable/memory"),
    )
    .quantity()
    .wide(),
    ColumnDef::new(
        "allocatable-pods",
        "Allocatable Pods",
        Int("/status/allocatable/pods"),
    )
    .number()
    .wide(),
    LABELS,
];

const NAMESPACE_KIND: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("status", "Status", Func(f::status_phase)),
    AGE,
    LABELS,
];

/// Events list the kubectl columns; the object's own (generated) name is a wide column.
const EVENT: &[ColumnDef] = &[
    NAMESPACE,
    ColumnDef::new("last-seen", "Last Seen", Func(f::event_last_seen)).age(),
    ColumnDef::new("type", "Type", Func(f::event_type)),
    ColumnDef::new("reason", "Reason", Text("/reason")),
    ColumnDef::new("object", "Object", Func(f::event_object)),
    ColumnDef::new("message", "Message", Text("/message")),
    ColumnDef::new("count", "Count", Int("/count")).number(),
    ColumnDef::new("source", "Source", Text("/source/component")).wide(),
    NAME.wide(),
    AGE.wide(),
];

const CRD: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("group", "Group", Text("/spec/group")),
    ColumnDef::new("version", "Version", Func(f::crd_version)),
    ColumnDef::new("scope", "Scope", Text("/spec/scope")),
    ColumnDef::new("short-names", "Short Names", Func(f::crd_short_names)),
    AGE,
    ColumnDef::new("kind", "Kind", Text("/spec/names/kind")).wide(),
    ColumnDef::new("resource", "Resource", Text("/spec/names/plural")).wide(),
];

const PRIORITY_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("value", "Value", Int("/value")).number(),
    ColumnDef::new("global-default", "Global Default", Text("/globalDefault")),
    AGE,
];

const RUNTIME_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("handler", "Handler", Text("/handler")),
    AGE,
];

const WEBHOOKS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("webhooks", "Webhooks", Len("/webhooks")).number(),
    AGE,
];

const ADMISSION_POLICY: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("validations", "Validations", Len("/spec/validations")).number(),
    ColumnDef::new("param-kind", "Param Kind", Text("/spec/paramKind/kind")),
    AGE,
];

const ADMISSION_POLICY_BINDING: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("policy", "Policy", Text("/spec/policyName")),
    ColumnDef::new("actions", "Actions", Func(f::binding_actions)),
    AGE,
];

pub(super) const KINDS: &[KindDef] = &[
    KindDef {
        group: "",
        kind: "Node",
        columns: NODE,
    },
    KindDef {
        group: "",
        kind: "Namespace",
        columns: NAMESPACE_KIND,
    },
    KindDef {
        group: "",
        kind: "Event",
        columns: EVENT,
    },
    KindDef {
        group: "apiextensions.k8s.io",
        kind: "CustomResourceDefinition",
        columns: CRD,
    },
    KindDef {
        group: "scheduling.k8s.io",
        kind: "PriorityClass",
        columns: PRIORITY_CLASS,
    },
    KindDef {
        group: "node.k8s.io",
        kind: "RuntimeClass",
        columns: RUNTIME_CLASS,
    },
    KindDef {
        group: "admissionregistration.k8s.io",
        kind: "MutatingWebhookConfiguration",
        columns: WEBHOOKS,
    },
    KindDef {
        group: "admissionregistration.k8s.io",
        kind: "ValidatingWebhookConfiguration",
        columns: WEBHOOKS,
    },
    KindDef {
        group: "admissionregistration.k8s.io",
        kind: "ValidatingAdmissionPolicy",
        columns: ADMISSION_POLICY,
    },
    KindDef {
        group: "admissionregistration.k8s.io",
        kind: "ValidatingAdmissionPolicyBinding",
        columns: ADMISSION_POLICY_BINDING,
    },
];
