//! The Pod columns, in display order, and their ids.

use std::sync::Arc;

use crate::columns::{Align, Column, ColumnId, SortKind};

/// `Ready`: `ready/total` containers.
pub const READY: &str = "ready";
/// `Status`: kubectl's status word.
pub const STATUS: &str = "status";
/// `Restarts`: the count and how long ago the last one was.
pub const RESTARTS: &str = "restarts";
/// `CPU`: usage (E13's metrics source).
pub const CPU: &str = "cpu";
/// `Memory`: usage (E13's metrics source).
pub const MEMORY: &str = "memory";
/// `%CPU/R`: CPU usage over the pod's CPU request.
pub const CPU_OF_REQUEST: &str = "cpu-request-pct";
/// `%CPU/L`: CPU usage over the pod's CPU limit.
pub const CPU_OF_LIMIT: &str = "cpu-limit-pct";
/// `%MEM/R`: memory usage over the pod's memory request.
pub const MEMORY_OF_REQUEST: &str = "memory-request-pct";
/// `%MEM/L`: memory usage over the pod's memory limit.
pub const MEMORY_OF_LIMIT: &str = "memory-limit-pct";
/// `IP`: the first pod IP.
pub const IP: &str = "ip";
/// `Node`: `spec.nodeName`.
pub const NODE: &str = "node";
/// `QoS`: `status.qosClass`.
pub const QOS: &str = "qos";
/// `Service Account`: `spec.serviceAccountName`.
pub const SERVICE_ACCOUNT: &str = "service-account";
/// `Controlled By`: the controller owner, `Kind/name`.
pub const CONTROLLED_BY: &str = "controlled-by";
/// `Nominated Node`: `status.nominatedNodeName` (`kubectl -o wide`).
pub const NOMINATED_NODE: &str = "nominated-node";
/// `Last Restart`: how long ago the last container restart finished (also in `Restarts`; kept
/// as its own column, the id the core catalogue gave it, so a saved layout that shows it keeps
/// it).
pub const LAST_RESTART: &str = "last-restart";

/// One column: id, title, tooltip, hidden by default, alignment, sort.
type Def = (
    &'static str,
    &'static str,
    Option<&'static str>,
    bool,
    Align,
    SortKind,
);

const L: Align = Align::Left;
const R: Align = Align::Right;

/// Lens' and k9s' pod columns in the story's order. Hidden by default, as Freelens hides them
/// (the only kind that sets default-hidden columns there): IP, Node and QoS; plus the two
/// `kubectl -o wide` / generic extras, Nominated Node, Last Restart and Labels.
const DEFS: &[Def] = &[
    (ColumnId::NAME, "Name", None, false, L, SortKind::Text),
    (
        ColumnId::NAMESPACE,
        "Namespace",
        None,
        false,
        L,
        SortKind::Text,
    ),
    (
        READY,
        "Ready",
        Some("Ready containers over containers (regular and sidecars)"),
        false,
        R,
        SortKind::Number,
    ),
    (
        STATUS,
        "Status",
        Some("The status kubectl get pods prints"),
        false,
        L,
        SortKind::Text,
    ),
    (
        RESTARTS,
        "Restarts",
        Some("Container restarts, and how long ago the last one was"),
        false,
        R,
        SortKind::Number,
    ),
    (
        CPU,
        "CPU",
        Some("CPU usage (metrics-server); – when there is no sample"),
        false,
        R,
        SortKind::Quantity,
    ),
    (
        MEMORY,
        "Memory",
        Some("Memory usage (metrics-server); – when there is no sample"),
        false,
        R,
        SortKind::Quantity,
    ),
    (
        CPU_OF_REQUEST,
        "%CPU/R",
        Some("CPU usage as a percentage of the pod's CPU request"),
        false,
        R,
        SortKind::Number,
    ),
    (
        CPU_OF_LIMIT,
        "%CPU/L",
        Some("CPU usage as a percentage of the pod's CPU limit"),
        false,
        R,
        SortKind::Number,
    ),
    (
        MEMORY_OF_REQUEST,
        "%MEM/R",
        Some("Memory usage as a percentage of the pod's memory request"),
        false,
        R,
        SortKind::Number,
    ),
    (
        MEMORY_OF_LIMIT,
        "%MEM/L",
        Some("Memory usage as a percentage of the pod's memory limit"),
        false,
        R,
        SortKind::Number,
    ),
    (IP, "IP", None, true, L, SortKind::Text),
    (NODE, "Node", None, true, L, SortKind::Text),
    (
        QOS,
        "QoS",
        Some("Quality of service class"),
        true,
        L,
        SortKind::Text,
    ),
    (
        SERVICE_ACCOUNT,
        "Service Account",
        None,
        false,
        L,
        SortKind::Text,
    ),
    (
        CONTROLLED_BY,
        "Controlled By",
        Some("The owner that controls the pod"),
        false,
        L,
        SortKind::Text,
    ),
    (ColumnId::AGE, "Age", None, false, R, SortKind::Age),
    (
        NOMINATED_NODE,
        "Nominated Node",
        None,
        true,
        L,
        SortKind::Text,
    ),
    (
        LAST_RESTART,
        "Last Restart",
        Some("How long ago the last container restart finished"),
        true,
        R,
        SortKind::Age,
    ),
    (ColumnId::LABELS, "Labels", None, true, L, SortKind::Text),
];

/// The columns, built once per provider.
pub(super) fn columns() -> Arc<[Column]> {
    DEFS.iter()
        .map(|&(id, title, description, wide, align, sort)| Column {
            id: ColumnId::new(id),
            title: Arc::from(title),
            description: description.map(Arc::from),
            wide,
            align,
            sort,
            table_index: None,
        })
        .collect()
}
