//! The graphs of each kind, as data: which requests make a chart and which reference lines it
//! carries. Six kinds, one table; the view draws whatever [`charts_for`] returns.

use oxikube_domain::{MetricKind, MetricRequest, MetricTarget, MetricUnit, PodSelector};

use super::subject::HistorySubject;

/// One measured line of a chart: a request and the name to give its series when the query does
/// not name them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    /// The legend name of an unlabelled result (`CPU`, `received`).
    pub name: &'static str,
    /// What to ask.
    pub request: MetricRequest,
}

/// A horizontal reference line source: the request or limit of the thing measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// What the line is called (`requests`, `limits`, `capacity`).
    pub label: &'static str,
    /// What to ask. Its last value is the line.
    pub request: MetricRequest,
}

/// One chart of the Metrics tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartDef {
    /// A stable id, also the chart's element id (`cpu`).
    pub id: &'static str,
    /// The chart's heading.
    pub title: &'static str,
    /// The unit of the values, which picks the axis format.
    pub unit: MetricUnit,
    /// The measured series.
    pub measures: Vec<Measure>,
    /// The requests, limits or capacity drawn as thresholds.
    pub references: Vec<Reference>,
}

fn measure(name: &'static str, target: &MetricTarget, kind: MetricKind) -> Measure {
    Measure {
        name,
        request: MetricRequest::new(target.clone(), kind),
    }
}

fn reference(label: &'static str, target: &MetricTarget, kind: MetricKind) -> Reference {
    Reference {
        label,
        request: MetricRequest::new(target.clone(), kind),
    }
}

fn cpu(target: &MetricTarget, references: bool) -> ChartDef {
    ChartDef {
        id: "cpu",
        title: "CPU",
        unit: MetricUnit::Cores,
        measures: vec![measure("CPU", target, MetricKind::CpuUsage)],
        references: if references {
            vec![
                reference("requests", target, MetricKind::CpuRequests),
                reference("limits", target, MetricKind::CpuLimits),
            ]
        } else {
            Vec::new()
        },
    }
}

fn memory(target: &MetricTarget, references: bool) -> ChartDef {
    ChartDef {
        id: "memory",
        title: "Memory",
        unit: MetricUnit::Bytes,
        measures: vec![measure("Memory", target, MetricKind::MemoryUsage)],
        references: if references {
            vec![
                reference("requests", target, MetricKind::MemoryRequests),
                reference("limits", target, MetricKind::MemoryLimits),
            ]
        } else {
            Vec::new()
        },
    }
}

fn network(target: &MetricTarget) -> ChartDef {
    ChartDef {
        id: "network",
        title: "Network",
        unit: MetricUnit::BytesPerSecond,
        measures: vec![
            measure("received", target, MetricKind::NetworkRx),
            measure("sent", target, MetricKind::NetworkTx),
        ],
        references: Vec::new(),
    }
}

/// The charts of `subject`, in the order the tab shows them. Empty for a subject with nothing to
/// query (a workload without pods).
pub fn charts_for(subject: &HistorySubject) -> Vec<ChartDef> {
    if subject.is_empty() {
        return Vec::new();
    }
    match subject {
        HistorySubject::Pod { namespace, name } => {
            // CPU and memory per container; traffic is counted per pod.
            let containers = MetricTarget::Container {
                namespace: namespace.clone(),
                pod: name.clone(),
                container: None,
            };
            let pod = MetricTarget::Pod {
                namespace: namespace.clone(),
                pods: PodSelector::One(name.clone()),
            };
            vec![
                cpu(&containers, true),
                memory(&containers, true),
                network(&pod),
            ]
        }
        HistorySubject::Workload {
            namespace, pods, ..
        } => {
            let target = MetricTarget::Pod {
                namespace: namespace.clone(),
                pods: PodSelector::Any(pods.clone()),
            };
            vec![cpu(&target, true), memory(&target, true), network(&target)]
        }
        HistorySubject::Node { name } => {
            let target = MetricTarget::Node(name.clone());
            let disk = ChartDef {
                id: "disk",
                title: "Disk",
                unit: MetricUnit::Bytes,
                measures: vec![measure("Disk used", &target, MetricKind::DiskUsage)],
                references: vec![reference("capacity", &target, MetricKind::DiskCapacity)],
            };
            vec![
                cpu(&target, true),
                memory(&target, true),
                network(&target),
                disk,
            ]
        }
        HistorySubject::Namespace { name } => {
            let target = MetricTarget::Namespace(name.clone());
            let pods = ChartDef {
                id: "pods",
                title: "Pods",
                unit: MetricUnit::Count,
                measures: vec![measure("pods", &target, MetricKind::PodCount)],
                references: Vec::new(),
            };
            vec![
                cpu(&target, true),
                memory(&target, true),
                network(&target),
                pods,
            ]
        }
        HistorySubject::Pvc { namespace, name } => {
            let target = MetricTarget::Pvc {
                namespace: namespace.clone(),
                name: name.clone(),
            };
            vec![ChartDef {
                id: "disk",
                title: "Disk",
                unit: MetricUnit::Bytes,
                measures: vec![measure("used", &target, MetricKind::DiskUsage)],
                references: vec![reference("capacity", &target, MetricKind::DiskCapacity)],
            }]
        }
        HistorySubject::Ingress { namespace, name } => {
            let target = MetricTarget::Ingress {
                namespace: namespace.clone(),
                name: name.clone(),
            };
            vec![ChartDef {
                id: "traffic",
                title: "Traffic",
                unit: MetricUnit::BytesPerSecond,
                measures: vec![
                    measure("requests in", &target, MetricKind::NetworkRx),
                    measure("responses out", &target, MetricKind::NetworkTx),
                ],
                references: Vec::new(),
            }]
        }
    }
}
