//! Which pod and pod port a forward goes to, pure: [`resolve`] over the [`TargetView`] the watch
//! keeps.
//!
//! * A **pod** target is usable while it exists, is `Running` and is not being deleted; a named
//!   remote port is looked up in its containers' ports (init containers included, for sidecars).
//! * A **Service** target goes to a pod behind a *ready* endpoint of its EndpointSlices
//!   (`discovery.k8s.io/v1`, label `kubernetes.io/service-name`), preferring the pod used before
//!   when it is still ready, else the first by name. The Service port (by number or name) maps
//!   to the pod port the way kube-proxy maps it: the slice port of the same name carries the
//!   resolved `targetPort` (the endpoint controller resolves a named `targetPort` per pod, and
//!   puts pods that resolve it differently in different slices); a slice without that port
//!   falls back to a numeric `targetPort`.

use oxikube_domain::Resource;
use oxikube_domain::portforward::ForwardPort;
use oxikube_domain::view::{EndpointSliceSummary, ServicePortSummary, ServiceSummary, TargetPort};

/// What the watch knows about a forward's target. `None` until the feed has listed.
#[derive(Debug, Clone, Default)]
pub struct TargetView {
    /// The target pod (a pod target), or nothing if it does not exist.
    pub pod: Option<Option<Resource>>,
    /// The Service (a Service target), or nothing if it does not exist.
    pub service: Option<Option<Resource>>,
    /// The Service's EndpointSlices.
    pub slices: Option<Vec<Resource>>,
}

impl TargetView {
    /// Whether every feed a target of this shape needs has listed.
    pub fn is_ready(&self, service: bool) -> bool {
        if service {
            self.service.is_some() && self.slices.is_some()
        } else {
            self.pod.is_some()
        }
    }
}

/// Where to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The pod's name (in the target's namespace).
    pub pod: String,
    /// The port on the pod.
    pub port: u16,
}

/// Why there is nowhere to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// Nothing ready yet; a watch event or a retry may fix it.
    Waiting(String),
    /// The target can never serve this port; the user must change the forward.
    Invalid(String),
}

/// Resolves `port` of the target in `view` (`service` says which kind), preferring `previous`.
pub fn resolve(
    view: &TargetView,
    service: bool,
    port: &ForwardPort,
    previous: Option<&str>,
) -> Result<Resolved, Unresolved> {
    if service {
        resolve_service(view, port, previous)
    } else {
        resolve_pod(view, port)
    }
}

fn resolve_pod(view: &TargetView, port: &ForwardPort) -> Result<Resolved, Unresolved> {
    let Some(Some(pod)) = &view.pod else {
        return Err(Unresolved::Waiting("the pod does not exist".into()));
    };
    if !pod_usable(pod) {
        return Err(Unresolved::Waiting(format!(
            "pod {} is not running",
            pod.meta.name
        )));
    }
    let number = match port {
        ForwardPort::Number(n) => *n,
        ForwardPort::Named(name) => container_port(pod, name).ok_or_else(|| {
            Unresolved::Invalid(format!("pod {} has no port named {name}", pod.meta.name))
        })?,
    };
    Ok(Resolved {
        pod: pod.meta.name.to_string(),
        port: number,
    })
}

fn resolve_service(
    view: &TargetView,
    port: &ForwardPort,
    previous: Option<&str>,
) -> Result<Resolved, Unresolved> {
    let Some(Some(service)) = &view.service else {
        return Err(Unresolved::Waiting("the Service does not exist".into()));
    };
    let summary = ServiceSummary::from_resource(service)
        .map_err(|err| Unresolved::Invalid(err.to_string()))?;
    let service_port = summary
        .ports
        .iter()
        .find(|p| match port {
            ForwardPort::Number(n) => p.port == *n,
            ForwardPort::Named(name) => p.name.as_deref() == Some(name.as_str()),
        })
        .ok_or_else(|| {
            Unresolved::Invalid(format!("Service {} has no port {port}", summary.name))
        })?;
    if !service_port.is_tcp() {
        return Err(Unresolved::Invalid(format!(
            "Service port {port} is {}; only TCP can be forwarded",
            service_port.protocol
        )));
    }
    let candidates = ready_pods(view.slices.as_deref().unwrap_or_default(), service_port);
    let chosen = previous
        .and_then(|p| candidates.iter().find(|(pod, _)| pod == p))
        .or_else(|| candidates.first())
        .cloned();
    match chosen {
        Some((pod, port)) => Ok(Resolved { pod, port }),
        None => Err(Unresolved::Waiting(format!(
            "Service {} has no ready pod",
            summary.name
        ))),
    }
}

/// The ready pods behind `service_port` with the pod port each serves it on, sorted by name.
fn ready_pods(slices: &[Resource], service_port: &ServicePortSummary) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = slices
        .iter()
        .filter_map(|res| EndpointSliceSummary::from_resource(res).ok())
        .flat_map(|slice| {
            let slice_port = slice
                .ports
                .iter()
                .find(|p| p.name == service_port.name)
                .and_then(|p| p.port);
            let number = slice_port.or(match service_port.target_port {
                TargetPort::Number(n) => Some(n),
                TargetPort::Named(_) => None,
            });
            slice
                .endpoints
                .into_iter()
                .filter(|e| e.ready && !e.terminating)
                .filter_map(|e| e.target)
                .filter(|t| &*t.kind == "Pod")
                .filter_map(move |t| Some((t.name.to_string(), number?)))
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Whether a pod can serve a forward: it is `Running` and not being deleted.
pub fn pod_usable(pod: &Resource) -> bool {
    let running = pod
        .get("/status/phase")
        .and_then(|p| p.as_str().map(|s| s == "Running"))
        .unwrap_or(false);
    running && pod.get("/metadata/deletionTimestamp").is_none()
}

/// The `containerPort` of the TCP container port `name` of `pod`.
fn container_port(pod: &Resource, name: &str) -> Option<u16> {
    let spec = pod.json().get("spec")?;
    ["containers", "initContainers"]
        .into_iter()
        .find_map(|key| {
            spec.get(key)?.as_array()?.iter().find_map(|container| {
                container.get("ports")?.as_array()?.iter().find_map(|p| {
                    let named = p.get("name").and_then(|n| n.as_str().map(|n| n == name));
                    let tcp = p
                        .get("protocol")
                        .and_then(|v| v.as_str().map(|s| s.eq_ignore_ascii_case("TCP")))
                        .unwrap_or(true);
                    if named == Some(true) && tcp {
                        u16::try_from(p.get("containerPort")?.as_i64()?).ok()
                    } else {
                        None
                    }
                })
            })
        })
}
