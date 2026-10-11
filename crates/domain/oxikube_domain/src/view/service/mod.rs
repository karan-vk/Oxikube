//! [`ServiceSummary`]: a Service as its table row and detail panel need it (E12-S13).
//!
//! The rules follow `kubectl get svc -o wide` (Apache-2.0 semantics, reimplemented): the port
//! text, the external address of each Service type and the selector. The load balancer state
//! behind the `STATUS` column is Lens' (`Pending` until the cloud assigns an address).

mod ports;

use std::fmt;
use std::sync::Arc;

pub use ports::{ServicePortSummary, TargetPort, resolve_target_port};

use super::{ViewError, arc_of, arr_of, bool_of, check_kind, str_of, sub};
use crate::json::JsonRef;
use crate::resource::Resource;

/// `spec.type`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServiceType {
    /// A virtual IP inside the cluster (the default).
    ClusterIP,
    /// A port on every node.
    NodePort,
    /// A cloud load balancer.
    LoadBalancer,
    /// A DNS alias; no proxying and no backing pods.
    ExternalName,
    /// A type this version does not know.
    Other(Arc<str>),
}

impl ServiceType {
    fn parse(text: Option<&str>) -> Self {
        match text {
            None | Some("" | "ClusterIP") => Self::ClusterIP,
            Some("NodePort") => Self::NodePort,
            Some("LoadBalancer") => Self::LoadBalancer,
            Some("ExternalName") => Self::ExternalName,
            Some(other) => Self::Other(Arc::from(other)),
        }
    }

    /// The API spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::ClusterIP => "ClusterIP",
            Self::NodePort => "NodePort",
            Self::LoadBalancer => "LoadBalancer",
            Self::ExternalName => "ExternalName",
            Self::Other(other) => other,
        }
    }
}

impl fmt::Display for ServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `STATUS` column: whether the Service is serving as configured. Only a load balancer has
/// a state to wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceStatus {
    /// Nothing to wait for.
    Active,
    /// A `LoadBalancer` Service whose address has not been assigned yet.
    Pending,
    /// A `LoadBalancer` Service with an address in `status.loadBalancer.ingress`.
    Assigned,
}

impl ServiceStatus {
    /// The text of the column.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Pending => "Pending",
            Self::Assigned => "Assigned",
        }
    }
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a Service lists no backing pods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoSelector {
    /// `ExternalName`: the name is a DNS alias to something outside the cluster.
    ExternalName,
    /// No `spec.selector`: whoever owns the Service manages its endpoints by hand.
    Manual,
}

impl NoSelector {
    /// The sentence the detail shows in place of the pods.
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::ExternalName => {
                "This is an ExternalName service: it is a DNS alias and has no backing pods."
            }
            Self::Manual => {
                "This service has no selector, so no pods are selected: its endpoints are \
                 managed by hand or by a controller."
            }
        }
    }
}

/// One Service, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `spec.type`.
    pub kind: ServiceType,
    /// `spec.clusterIPs` (one per IP family on a dual-stack cluster), else `spec.clusterIP`;
    /// empty for a headless Service.
    pub cluster_ips: Vec<Arc<str>>,
    /// Whether the Service is headless (`clusterIP: None`).
    pub headless: bool,
    /// `spec.ipFamilies`.
    pub ip_families: Vec<Arc<str>>,
    /// `spec.externalName`.
    pub external_name: Option<Arc<str>>,
    /// `spec.externalIPs`.
    pub external_ips: Vec<Arc<str>>,
    /// `status.loadBalancer.ingress` addresses (`ip`, else `hostname`).
    pub load_balancer: Vec<Arc<str>>,
    /// `spec.ports`.
    pub ports: Vec<ServicePortSummary>,
    /// `spec.selector`, sorted by key.
    pub selector: Vec<(Arc<str>, Arc<str>)>,
    /// `spec.sessionAffinity` (`None` or `ClientIP`).
    pub session_affinity: Arc<str>,
    /// `spec.publishNotReadyAddresses`: endpoints that are not ready still receive traffic.
    pub publish_not_ready: bool,
    /// The `STATUS` column.
    pub status: ServiceStatus,
}

impl ServiceSummary {
    /// Reads a core `v1` Service.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind. Missing fields degrade to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Service", &[("", "Service")])?;
        let spec = sub(res.json(), "spec");
        let kind = ServiceType::parse(str_of(spec, "type"));
        let strings = |key: &str| -> Vec<Arc<str>> {
            arr_of(spec, key)
                .iter()
                .filter_map(JsonRef::as_str)
                .filter(|s| !s.is_empty())
                .map(Arc::from)
                .collect()
        };
        let primary = str_of(spec, "clusterIP");
        let headless = primary == Some("None");
        let mut cluster_ips = strings("clusterIPs");
        if cluster_ips.is_empty() {
            cluster_ips.extend(primary.map(Arc::from));
        }
        if headless {
            cluster_ips.retain(|ip| &**ip != "None");
        }
        let load_balancer: Vec<Arc<str>> =
            arr_of(sub(sub(res.json(), "status"), "loadBalancer"), "ingress")
                .iter()
                .filter_map(|i| str_of(i, "ip").or_else(|| str_of(i, "hostname")))
                .map(Arc::from)
                .collect();
        let status = match (&kind, load_balancer.is_empty()) {
            (ServiceType::LoadBalancer, true) => ServiceStatus::Pending,
            (ServiceType::LoadBalancer, false) => ServiceStatus::Assigned,
            _ => ServiceStatus::Active,
        };
        let mut selector: Vec<(Arc<str>, Arc<str>)> = spec
            .get("selector")
            .and_then(JsonRef::as_object)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (Arc::from(k), Arc::from(v.as_str().unwrap_or_default())))
                    .collect()
            })
            .unwrap_or_default();
        selector.sort();
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            kind,
            cluster_ips,
            headless,
            ip_families: strings("ipFamilies"),
            external_name: arc_of(spec, "externalName"),
            external_ips: strings("externalIPs"),
            load_balancer,
            ports: arr_of(spec, "ports")
                .iter()
                .filter_map(ServicePortSummary::from_json)
                .collect(),
            selector,
            session_affinity: arc_of(spec, "sessionAffinity").unwrap_or_else(|| Arc::from("None")),
            publish_not_ready: bool_of(spec, "publishNotReadyAddresses"),
            status,
        })
    }

    /// The `CLUSTER-IP` column: the primary cluster IP, `None` for a headless Service, empty for
    /// an ExternalName one.
    pub fn cluster_ip_text(&self) -> &str {
        if self.headless {
            "None"
        } else {
            self.cluster_ips.first().map_or("", |ip| ip)
        }
    }

    /// The `EXTERNAL-IP` column, by kubectl's rules: an ExternalName Service shows its name; a
    /// load balancer its assigned addresses and `spec.externalIPs`, or `<pending>` while it has
    /// neither; any other type its `spec.externalIPs`. Empty when there is none.
    pub fn external_ip_text(&self) -> String {
        match self.kind {
            ServiceType::ExternalName => {
                self.external_name.as_deref().unwrap_or_default().to_owned()
            }
            ServiceType::LoadBalancer => {
                let all: Vec<&str> = self
                    .load_balancer
                    .iter()
                    .chain(&self.external_ips)
                    .map(|a| &**a)
                    .collect();
                if all.is_empty() {
                    "<pending>".to_owned()
                } else {
                    all.join(",")
                }
            }
            _ => self.external_ips.join(","),
        }
    }

    /// The `PORT(S)` column: every port as `80/TCP` or `80:30080/TCP`, comma-separated. Empty
    /// when the Service has no ports (a headless one often has none).
    pub fn ports_text(&self) -> String {
        let mut out = String::new();
        for port in &self.ports {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(&port.kubectl_text());
        }
        out
    }

    /// The `SELECTOR` column: `k=v,k=v`, the label selector the pods are queried with. Empty
    /// when the Service selects nothing.
    pub fn selector_text(&self) -> String {
        let mut out = String::new();
        for (key, value) in &self.selector {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(key);
            out.push('=');
            out.push_str(value);
        }
        out
    }

    /// The label selector of the Service's backing pods, `None` when it has none to query with
    /// (ExternalName, or no `spec.selector`): the reason is [`no_selector`](Self::no_selector).
    pub fn pod_selector(&self) -> Option<String> {
        self.no_selector().is_none().then(|| self.selector_text())
    }

    /// Why the Service has no backing pods, `None` when it selects some.
    pub fn no_selector(&self) -> Option<NoSelector> {
        if self.kind == ServiceType::ExternalName {
            Some(NoSelector::ExternalName)
        } else if self.selector.is_empty() {
            Some(NoSelector::Manual)
        } else {
            None
        }
    }

    /// The ports a browser can reach through a port-forward: the TCP ones.
    pub fn browsable_ports(&self) -> impl Iterator<Item = &ServicePortSummary> {
        self.ports.iter().filter(|p| p.is_tcp())
    }
}
