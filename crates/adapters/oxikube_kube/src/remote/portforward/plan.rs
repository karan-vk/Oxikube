//! What a forward resolves to, as pure data: the pod and port to dial, chosen from a snapshot
//! of the candidate pods. Nothing here touches the network.

use std::collections::BTreeMap;

use oxikube_domain::{ForwardPort, OxiError};

/// A pod as far as forwarding cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodInfo {
    pub(super) name: String,
    /// `status.phase == Running`.
    pub(super) running: bool,
    /// The `Ready` condition is `True`.
    pub(super) ready: bool,
    /// `metadata.deletionTimestamp` is set.
    pub(super) terminating: bool,
    /// Creation time, seconds since the epoch; orders replicas deterministically.
    pub(super) created: i64,
    /// TCP container ports.
    pub(super) ports: Vec<ContainerPort>,
}

/// One TCP port a container declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContainerPort {
    pub(super) name: Option<String>,
    pub(super) number: u16,
}

impl PodInfo {
    /// The number of the container port called `name`.
    fn named_port(&self, name: &str) -> Option<u16> {
        self.ports
            .iter()
            .find(|p| p.name.as_deref() == Some(name))
            .map(|p| p.number)
    }
}

/// Where a service port points on a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TargetPort {
    Number(u16),
    /// A container port name, resolved per pod.
    Name(String),
}

/// One entry of `spec.ports` of a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServicePort {
    pub(super) name: Option<String>,
    pub(super) port: u16,
    pub(super) target: TargetPort,
}

/// The parts of a service forwarding needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceInfo {
    pub(super) selector: BTreeMap<String, String>,
    pub(super) ports: Vec<ServicePort>,
}

/// The pod and port a forward currently dials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) pod: String,
    pub(super) port: u16,
}

/// Which pods can serve a forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PodSelector {
    /// Exactly this pod.
    Name(String),
    /// Every pod carrying these labels.
    Labels(BTreeMap<String, String>),
}

impl PodSelector {
    /// The `fieldSelector` query value, when the selector is by name.
    pub(super) fn field_selector(&self) -> Option<String> {
        match self {
            Self::Name(name) => Some(format!("metadata.name={name}")),
            Self::Labels(_) => None,
        }
    }

    /// The `labelSelector` query value, when the selector is by labels.
    pub(super) fn label_selector(&self) -> Option<String> {
        match self {
            Self::Name(_) => None,
            Self::Labels(labels) => Some(
                labels
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        }
    }
}

/// A forward's resolution rules, fixed when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Plan {
    /// A named pod. It serves while it is running and not terminating (readiness is not
    /// required: someone forwarding to a pod usually wants to look at a sick one).
    Pod {
        label: String,
        name: String,
        port: ForwardPort,
    },
    /// The ready pods behind a service.
    Service {
        label: String,
        selector: BTreeMap<String, String>,
        port: ServicePort,
    },
}

impl Plan {
    /// A plan for the pod `namespace/name`.
    pub(super) fn pod(namespace: &str, name: &str, port: ForwardPort) -> Self {
        Self::Pod {
            label: format!("{namespace}/{name}"),
            name: name.to_owned(),
            port,
        }
    }

    /// A plan for `namespace/name`, mapping `remote` (a service port number or name) through
    /// `service`.
    pub(super) fn service(
        namespace: &str,
        name: &str,
        service: ServiceInfo,
        remote: &ForwardPort,
    ) -> Result<Self, OxiError> {
        let label = format!("{namespace}/{name}");
        if service.selector.is_empty() {
            return Err(OxiError::validation(format!(
                "service {label} has no selector, so there are no pods to forward to"
            )));
        }
        let port = service
            .ports
            .iter()
            .find(|p| match remote {
                ForwardPort::Number(n) => p.port == *n,
                ForwardPort::Named(n) => p.name.as_deref() == Some(n),
            })
            .cloned()
            .ok_or_else(|| {
                let offered = service
                    .ports
                    .iter()
                    .map(|p| match &p.name {
                        Some(name) => format!("{}/{name}", p.port),
                        None => p.port.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                OxiError::validation(format!(
                    "service {label} has no port {remote} (it serves: {offered})"
                ))
            })?;
        Ok(Self::Service {
            label,
            selector: service.selector,
            port,
        })
    }

    /// Whether the forward follows a service (and so may move between pods).
    pub(super) fn is_service(&self) -> bool {
        matches!(self, Self::Service { .. })
    }

    /// The pods to watch.
    pub(super) fn selector(&self) -> PodSelector {
        match self {
            Self::Pod { name, .. } => PodSelector::Name(name.clone()),
            Self::Service { selector, .. } => PodSelector::Labels(selector.clone()),
        }
    }

    /// The pod port on `pod`, `None` when a named port is not declared there.
    fn port_on(&self, pod: &PodInfo) -> Option<u16> {
        match self {
            Self::Pod { port, .. } => match port {
                ForwardPort::Number(n) => Some(*n),
                ForwardPort::Named(name) => pod.named_port(name),
            },
            Self::Service { port, .. } => match &port.target {
                TargetPort::Number(n) => Some(*n),
                TargetPort::Name(name) => pod.named_port(name),
            },
        }
    }

    fn state_allows(&self, pod: &PodInfo) -> bool {
        pod.running && !pod.terminating && (!self.is_service() || pod.ready)
    }

    /// What to dial on `pod`, `None` when `pod` cannot serve right now.
    pub(super) fn target_on(&self, pod: &PodInfo) -> Option<Target> {
        if !self.state_allows(pod) {
            return None;
        }
        Some(Target {
            pod: pod.name.clone(),
            port: self.port_on(pod)?,
        })
    }

    /// The pod to forward to: the oldest one that can serve (name breaks ties), so the choice
    /// does not flap between equivalent replicas.
    pub(super) fn pick(&self, pods: &[PodInfo]) -> Option<Target> {
        pods.iter()
            .filter(|pod| self.target_on(pod).is_some())
            .min_by(|a, b| (a.created, &a.name).cmp(&(b.created, &b.name)))
            .and_then(|pod| self.target_on(pod))
    }

    /// Why [`pick`](Self::pick) found nothing, as the error a caller of `start` sees.
    pub(super) fn why_no_target(&self, pods: &[PodInfo]) -> OxiError {
        match self {
            Self::Pod { label, name, port } => match pods.iter().find(|p| &p.name == name) {
                None => OxiError::not_found(format!("pod {label} not found")),
                Some(pod) if pod.terminating => {
                    OxiError::conflict(format!("pod {label} is terminating"))
                }
                Some(pod) if !pod.running => {
                    OxiError::conflict(format!("pod {label} is not running"))
                }
                Some(_) => {
                    OxiError::validation(format!("pod {label} declares no container port {port}"))
                }
            },
            Self::Service { label, port, .. } => {
                let ready = pods.iter().filter(|p| self.state_allows(p)).count();
                let message = if pods.is_empty() {
                    format!("service {label} has no pods behind its selector")
                } else if ready == 0 {
                    format!(
                        "none of the {} pods behind service {label} is ready",
                        pods.len()
                    )
                } else {
                    let target = match &port.target {
                        TargetPort::Name(name) => name.clone(),
                        TargetPort::Number(n) => n.to_string(),
                    };
                    format!("no ready pod behind service {label} declares port {target}")
                };
                // Pods come and go: asking again later may work.
                OxiError::not_found(message).with_retryable(true)
            }
        }
    }
}
