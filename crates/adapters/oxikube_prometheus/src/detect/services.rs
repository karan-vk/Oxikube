//! Listing the Services of a candidate namespace and choosing the port to query.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_domain::redact::redact;
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::{ListOptions, ResourceReader, ServicePort};

/// A Service port as `spec.ports` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PortInfo {
    pub(super) name: Option<String>,
    pub(super) number: u16,
}

/// What the detector needs to know about a Service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceInfo {
    pub(super) name: String,
    pub(super) ports: Vec<PortInfo>,
}

impl ServiceInfo {
    fn from_resource(service: &Resource) -> Option<Self> {
        // An ExternalName Service has no endpoints for the proxy to forward to.
        if service.get_str("/spec/type") == Some("ExternalName") {
            return None;
        }
        let ports = service
            .get("/spec/ports")
            .and_then(|ports| ports.as_array())
            .map(|ports| {
                ports
                    .iter()
                    .filter_map(|port| {
                        let number = u16::try_from(port.get("port")?.as_i64()?).ok()?;
                        Some(PortInfo {
                            name: port.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                            number,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            name: service.name().to_owned(),
            ports,
        })
    }

    /// The port to query: the first of `preferred` (names or numbers) the Service exposes; the
    /// only port when it exposes one and none of the preferred match; `None` otherwise.
    pub(super) fn pick_port(&self, preferred: &[String]) -> Option<ServicePort> {
        for want in preferred {
            let by_number = want.parse::<u16>().ok();
            let hit = self.ports.iter().find(|port| {
                Some(port.number) == by_number || port.name.as_deref() == Some(want.as_str())
            });
            if let Some(port) = hit {
                return Some(match &port.name {
                    Some(name) => ServicePort::Name(name.clone()),
                    None => ServicePort::Number(port.number),
                });
            }
        }
        match self.ports.as_slice() {
            [only] => Some(ServicePort::Number(only.number)),
            _ => None,
        }
    }
}

/// What listing one namespace gave.
#[derive(Debug)]
pub(super) enum Listing {
    /// The Services found (possibly none).
    Services(Vec<ServiceInfo>),
    /// The list failed.
    Failed {
        /// RBAC refused it.
        forbidden: bool,
        /// A sentence, safe to show.
        reason: String,
    },
}

/// The most Services read from one namespace. A namespace holding more than this does not hold
/// a metrics stack of ours among a page's worth of unrelated Services in any realistic cluster.
const PAGE: u32 = 500;
/// Per-list deadline.
const LIST_TIMEOUT_SECS: u32 = 15;

/// Lists the Services in `namespace`.
pub(super) async fn list(resources: &dyn ResourceReader, namespace: &str) -> Listing {
    let kind = Gvk::new("", "v1", "Service");
    let options = ListOptions {
        limit: Some(PAGE),
        timeout_secs: Some(LIST_TIMEOUT_SECS),
        ..ListOptions::default()
    };
    match resources.list(&kind, Some(namespace), &options).await {
        Ok(page) => Listing::Services(
            page.items
                .iter()
                .filter_map(ServiceInfo::from_resource)
                .collect(),
        ),
        Err(error) => Listing::Failed {
            forbidden: error.kind() == ErrorKind::Forbidden,
            reason: describe(&error),
        },
    }
}

fn describe(error: &OxiError) -> String {
    match error.kind() {
        ErrorKind::Forbidden => "you may not list services here".to_owned(),
        ErrorKind::Timeout => "listing services timed out".to_owned(),
        _ => redact(error.message())
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    }
}
