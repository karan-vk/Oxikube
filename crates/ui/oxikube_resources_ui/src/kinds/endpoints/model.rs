//! [`EndpointsView`]: what the Endpoints and EndpointSlice panels draw, built from the object the
//! detail holds. Plain Rust, no GPUI.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::view::{
    EndpointSliceSummary, EndpointSummary, EndpointTarget, EndpointsSummary, ReadyCounts,
};

/// The most endpoints listed; the counts cover them all.
pub(super) const MAX_ENDPOINTS: usize = 100;

/// One endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EndpointRow {
    pub addresses: String,
    /// `Ready`, `Not ready` or `Terminating`.
    pub state: &'static str,
    pub tone: Tone,
    pub node: String,
    /// The pod (or other object) behind it, as a link.
    pub target: Option<(String, ResourceRef)>,
}

/// The panel's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EndpointsView {
    /// The Service these endpoints belong to, as a link.
    pub service: Option<ResourceRef>,
    /// `IPv4`, or empty for the core `Endpoints` object.
    pub address_type: String,
    pub ports: String,
    pub counts: ReadyCounts,
    pub rows: Vec<EndpointRow>,
    /// Endpoints beyond [`MAX_ENDPOINTS`].
    pub more: usize,
}

impl EndpointsView {
    /// The view of an EndpointSlice or an `Endpoints` object; `None` for any other kind.
    pub(super) fn of(resource: &Resource, cluster: &ClusterId) -> Option<Self> {
        if let Ok(slice) = EndpointSliceSummary::from_resource(resource) {
            let service = slice
                .service
                .as_ref()
                .map(|name| service_ref(cluster, &slice.namespace, name));
            let rows = slice
                .endpoints
                .iter()
                .take(MAX_ENDPOINTS)
                .map(|e| slice_row(e, cluster, &slice.namespace))
                .collect();
            return Some(Self {
                service,
                address_type: slice.address_type.to_string(),
                ports: slice.ports_text(),
                counts: slice.ready_counts(),
                more: slice.endpoints.len().saturating_sub(MAX_ENDPOINTS),
                rows,
            });
        }
        let endpoints = EndpointsSummary::from_resource(resource).ok()?;
        // An Endpoints object is named after its Service.
        let service = Some(service_ref(cluster, &endpoints.namespace, &endpoints.name));
        let ports = endpoints
            .ports
            .iter()
            .map(|p| p.text())
            .collect::<Vec<_>>()
            .join(",");
        let rows = endpoints
            .addresses
            .iter()
            .take(MAX_ENDPOINTS)
            .map(|a| EndpointRow {
                addresses: a.ip.to_string(),
                state: if a.ready { "Ready" } else { "Not ready" },
                tone: if a.ready { Tone::Ok } else { Tone::Warn },
                node: a.node.as_deref().unwrap_or_default().to_owned(),
                target: a
                    .target
                    .as_ref()
                    .and_then(|t| link(t, cluster, &endpoints.namespace)),
            })
            .collect();
        Some(Self {
            service,
            address_type: String::new(),
            ports,
            counts: endpoints.ready_counts(),
            more: endpoints.addresses.len().saturating_sub(MAX_ENDPOINTS),
            rows,
        })
    }
}

fn service_ref(cluster: &ClusterId, namespace: &Option<Arc<str>>, name: &Arc<str>) -> ResourceRef {
    ResourceRef::new(
        cluster.clone(),
        Gvk::new("", "v1", "Service"),
        namespace.clone(),
        name.clone(),
    )
}

fn slice_row(
    endpoint: &EndpointSummary,
    cluster: &ClusterId,
    namespace: &Option<Arc<str>>,
) -> EndpointRow {
    let (state, tone) = if endpoint.terminating {
        ("Terminating", Tone::Warn)
    } else if endpoint.ready {
        ("Ready", Tone::Ok)
    } else {
        ("Not ready", Tone::Warn)
    };
    EndpointRow {
        addresses: endpoint
            .addresses
            .iter()
            .map(|a| &**a)
            .collect::<Vec<_>>()
            .join(", "),
        state,
        tone,
        node: endpoint.node.as_deref().unwrap_or_default().to_owned(),
        target: endpoint
            .target
            .as_ref()
            .and_then(|t| link(t, cluster, namespace)),
    }
}

/// A link to what an endpoint points at: a pod, in the endpoint's namespace unless it names one.
fn link(
    target: &EndpointTarget,
    cluster: &ClusterId,
    namespace: &Option<Arc<str>>,
) -> Option<(String, ResourceRef)> {
    if &*target.kind != "Pod" {
        return None;
    }
    let reference = ResourceRef::new(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        target.namespace.clone().or_else(|| namespace.clone()),
        target.name.clone(),
    );
    Some((target.name.to_string(), reference))
}
