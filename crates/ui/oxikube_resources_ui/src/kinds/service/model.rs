//! [`ServiceView`]: everything the Service panel draws, computed off the render path from the
//! Service, its EndpointSlices and its pods. Plain Rust, no GPUI.
//!
//! The lists are capped ([`MAX_SLICES`], [`MAX_PODS`]) with a "N more" line, so a Service with a
//! thousand pods costs the same to draw as one with ten. The counts always cover every object.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;
use oxikube_app::{ColumnId, ColumnProvider};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::{
    EndpointSliceSummary, NoSelector, ReadyCounts, ServiceSummary, ServiceType, ready_counts,
    resolve_target_port,
};

/// The most EndpointSlices listed.
pub(super) const MAX_SLICES: usize = 20;
/// The most backing pods listed.
pub(super) const MAX_PODS: usize = 50;

/// How one of the panel's two feeds stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FeedStatus {
    /// The store has not answered yet.
    Loading,
    /// Answered (the list may be empty).
    Ready,
    /// The feed cannot be served (forbidden, failed): why.
    Failed(String),
}

/// One port of the Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PortRow {
    /// `http (80)` or `80`.
    pub label: String,
    /// The Service port.
    pub port: u16,
    /// kubectl's text: `80/TCP`, `80:30080/TCP`.
    pub text: String,
    /// What it reaches on a pod: `8080`, or `http → 8080` once a pod names it, or `http`.
    pub target: String,
    /// Whether "Open in browser" applies: TCP only.
    pub browsable: bool,
}

/// One EndpointSlice of the Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SliceRow {
    pub target: ResourceRef,
    pub name: Arc<str>,
    pub address_type: String,
    pub ports: String,
    pub counts: ReadyCounts,
}

/// One backing pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodRow {
    pub target: ResourceRef,
    pub name: Arc<str>,
    pub status: String,
    pub tone: Tone,
    pub ready: String,
    pub ip: String,
}

/// The EndpointSlices section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Slices {
    Loading,
    Failed(String),
    Listed {
        rows: Vec<SliceRow>,
        /// Slices beyond [`MAX_SLICES`].
        more: usize,
        counts: ReadyCounts,
    },
}

/// The backing pods section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pods {
    Loading,
    Failed(String),
    /// The Service selects no pods, and why.
    None(NoSelector),
    Listed {
        rows: Vec<PodRow>,
        /// Pods beyond [`MAX_PODS`].
        more: usize,
        total: usize,
    },
}

/// The panel's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceView {
    /// `(label, value)` facts: type, addresses, affinity.
    pub facts: Vec<(&'static str, String)>,
    /// The selector as `key=value` chips.
    pub selector: Vec<String>,
    pub ports: Vec<PortRow>,
    pub slices: Slices,
    pub pods: Pods,
    /// A line about whether traffic reaches anything.
    pub health: Option<(String, Tone)>,
}

/// What [`ServiceView::build`] reads.
pub(super) struct Inputs<'a> {
    pub service: &'a ServiceSummary,
    pub target: &'a ResourceRef,
    pub slices: &'a [Arc<StoreObject>],
    pub slices_status: &'a FeedStatus,
    /// `None`: the Service selects no pods.
    pub pods: Option<&'a [Arc<StoreObject>]>,
    pub pods_status: &'a FeedStatus,
    pub columns: &'a dyn ColumnProvider,
    pub now: Timestamp,
}

impl ServiceView {
    pub(super) fn build(inputs: &Inputs<'_>) -> Self {
        let Inputs { service, .. } = inputs;
        let pod_resources: Vec<&oxikube_domain::Resource> = inputs
            .pods
            .unwrap_or_default()
            .iter()
            .filter_map(|o| o.resource())
            .collect();
        let summaries: Vec<EndpointSliceSummary> = inputs
            .slices
            .iter()
            .filter_map(|o| o.resource())
            .filter_map(|r| EndpointSliceSummary::from_resource(r).ok())
            .collect();
        let counts = ready_counts(&summaries);

        let ports = service
            .ports
            .iter()
            .map(|port| {
                let resolved = resolve_target_port(port, pod_resources.iter().copied());
                let target = match (&port.target_port, resolved) {
                    (oxikube_domain::view::TargetPort::Named(name), Some(n)) => {
                        format!("{name} → {n}")
                    }
                    (target, _) => target.to_string(),
                };
                PortRow {
                    label: port.label(),
                    port: port.port,
                    text: port.kubectl_text(),
                    target,
                    browsable: port.is_tcp(),
                }
            })
            .collect();

        let slices = match inputs.slices_status {
            FeedStatus::Loading => Slices::Loading,
            FeedStatus::Failed(why) => Slices::Failed(why.clone()),
            FeedStatus::Ready => Slices::Listed {
                rows: summaries
                    .iter()
                    .take(MAX_SLICES)
                    .map(|s| slice_row(inputs.target, s))
                    .collect(),
                more: summaries.len().saturating_sub(MAX_SLICES),
                counts,
            },
        };

        let pods = match inputs.pods {
            None => Pods::None(service.no_selector().unwrap_or(NoSelector::Manual)),
            Some(rows) => match inputs.pods_status {
                FeedStatus::Loading => Pods::Loading,
                FeedStatus::Failed(why) => Pods::Failed(why.clone()),
                FeedStatus::Ready => Pods::Listed {
                    rows: rows
                        .iter()
                        .take(MAX_PODS)
                        .map(|o| pod_row(inputs, o))
                        .collect(),
                    more: rows.len().saturating_sub(MAX_PODS),
                    total: rows.len(),
                },
            },
        };

        let health =
            (*inputs.slices_status == FeedStatus::Ready).then(|| health_line(service, counts));
        Self {
            facts: facts(service),
            selector: service
                .selector
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect(),
            ports,
            slices,
            pods,
            health,
        }
    }
}

fn health_line(service: &ServiceSummary, counts: ReadyCounts) -> (String, Tone) {
    let mut text = format!("Endpoints: {}", counts.summary());
    let tone = if service.kind == ServiceType::ExternalName {
        return ("Endpoints: none (ExternalName)".to_owned(), Tone::Neutral);
    } else if counts.total() == 0 {
        Tone::Warn
    } else if counts.ready == 0 {
        Tone::Error
    } else if counts.not_ready > 0 {
        Tone::Warn
    } else {
        Tone::Ok
    };
    if service.publish_not_ready && counts.not_ready > 0 {
        text.push_str(" (not-ready endpoints still receive traffic)");
    }
    (text, tone)
}

fn facts(service: &ServiceSummary) -> Vec<(&'static str, String)> {
    let mut facts = vec![("Type", service.kind.to_string())];
    if service.headless {
        facts.push(("Cluster IP", "None (headless)".to_owned()));
    } else if !service.cluster_ips.is_empty() {
        let ips: Vec<&str> = service.cluster_ips.iter().map(|ip| &**ip).collect();
        facts.push((
            if ips.len() > 1 {
                "Cluster IPs"
            } else {
                "Cluster IP"
            },
            ips.join(", "),
        ));
    }
    // One family is the default and the cluster IP says it; two is worth a line.
    if service.ip_families.len() > 1 {
        let families: Vec<&str> = service.ip_families.iter().map(|f| &**f).collect();
        facts.push(("IP families", families.join(", ")));
    }
    let external = service.external_ip_text();
    if !external.is_empty() {
        facts.push(("External", external));
    }
    // The status is the chip in the detail's header.
    facts.push(("Session affinity", service.session_affinity.to_string()));
    if service.publish_not_ready {
        facts.push(("Publish not ready", "true".to_owned()));
    }
    facts
}

fn slice_row(service: &ResourceRef, slice: &EndpointSliceSummary) -> SliceRow {
    SliceRow {
        target: ResourceRef::new(
            service.cluster.clone(),
            Gvk::new("discovery.k8s.io", "v1", "EndpointSlice"),
            slice.namespace.as_deref().map(Arc::from),
            slice.name.clone(),
        ),
        name: slice.name.clone(),
        address_type: slice.address_type.to_string(),
        ports: slice.ports_text(),
        counts: slice.ready_counts(),
    }
}

fn pod_row(inputs: &Inputs<'_>, object: &StoreObject) -> PodRow {
    let cell = |id: &str| inputs.columns.cell(object, &ColumnId::new(id), inputs.now);
    let status = cell("status");
    let meta = object.meta();
    PodRow {
        target: ResourceRef::new(
            inputs.target.cluster.clone(),
            Gvk::new("", "v1", "Pod"),
            meta.namespace.clone(),
            meta.name.clone(),
        ),
        name: meta.name.clone(),
        status: status.display().to_owned(),
        tone: status.tone(),
        ready: cell("ready").display().to_owned(),
        ip: cell("ip").display().to_owned(),
    }
}
