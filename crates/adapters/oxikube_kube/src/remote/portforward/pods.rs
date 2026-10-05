//! `k8s-openapi` objects to the forwarding model, and the pod set folded from watch events.
//! kube types stop here (ADR 0005).

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::runtime::watcher::Event;

use super::plan::{ContainerPort, PodInfo, ServiceInfo, ServicePort, TargetPort};

/// `None` for a pod without a name (the server never sends one).
pub(super) fn pod_info(pod: &Pod) -> Option<PodInfo> {
    let name = pod.metadata.name.clone()?;
    let status = pod.status.as_ref();
    let ports = pod
        .spec
        .iter()
        .flat_map(|spec| &spec.containers)
        .flat_map(|container| container.ports.iter().flatten())
        .filter(|port| matches!(port.protocol.as_deref(), None | Some("TCP")))
        .filter_map(|port| {
            Some(ContainerPort {
                name: port.name.clone(),
                number: u16::try_from(port.container_port).ok()?,
            })
        })
        .collect();
    Some(PodInfo {
        name,
        running: status.and_then(|s| s.phase.as_deref()) == Some("Running"),
        ready: status
            .and_then(|s| s.conditions.as_ref())
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            }),
        terminating: pod.metadata.deletion_timestamp.is_some(),
        created: pod
            .metadata
            .creation_timestamp
            .as_ref()
            .map_or(0, |t| t.0.as_second()),
        ports,
    })
}

/// The selector and ports of a service.
pub(super) fn service_info(service: &Service) -> ServiceInfo {
    let spec = service.spec.as_ref();
    let ports = spec
        .and_then(|s| s.ports.as_ref())
        .into_iter()
        .flatten()
        .filter(|p| matches!(p.protocol.as_deref(), None | Some("TCP")))
        .filter_map(|p| {
            let port = u16::try_from(p.port).ok()?;
            let target = match &p.target_port {
                None => TargetPort::Number(port),
                Some(IntOrString::Int(n)) => TargetPort::Number(u16::try_from(*n).ok()?),
                Some(IntOrString::String(name)) => TargetPort::Name(name.clone()),
            };
            Some(ServicePort {
                name: p.name.clone().filter(|n| !n.is_empty()),
                port,
                target,
            })
        })
        .collect();
    ServiceInfo {
        selector: spec.and_then(|s| s.selector.clone()).unwrap_or_default(),
        ports,
    }
}

/// The current pods of a watch, rebuilt from its events.
#[derive(Default)]
pub(super) struct PodSet {
    pods: BTreeMap<String, PodInfo>,
    /// Pods listed so far by a (re)start of the watch, swapped in at `InitDone` so a relist
    /// never shows a half-empty set.
    relist: Option<BTreeMap<String, PodInfo>>,
}

impl PodSet {
    /// Applies one event; the new snapshot when the visible set changed.
    pub(super) fn apply(&mut self, event: Event<Pod>) -> Option<Vec<PodInfo>> {
        match event {
            Event::Init => {
                self.relist = Some(BTreeMap::new());
                None
            }
            Event::InitApply(pod) => {
                if let (Some(relist), Some(info)) = (self.relist.as_mut(), pod_info(&pod)) {
                    relist.insert(info.name.clone(), info);
                }
                None
            }
            Event::InitDone => {
                self.pods = self.relist.take()?;
                Some(self.snapshot())
            }
            Event::Apply(pod) => {
                let info = pod_info(&pod)?;
                self.pods.insert(info.name.clone(), info);
                Some(self.snapshot())
            }
            Event::Delete(pod) => {
                let name = pod.metadata.name?;
                self.pods.remove(&name)?;
                Some(self.snapshot())
            }
        }
    }

    fn snapshot(&self) -> Vec<PodInfo> {
        self.pods.values().cloned().collect()
    }
}
