//! [`IngressSummary`]: an Ingress as the rules table shows it.

use std::fmt;
use std::sync::Arc;

use super::super::{ViewError, arr_of, check_kind, str_of, sub};
use crate::json::JsonRef;
use crate::resource::Resource;

/// The legacy class annotation, read when `spec.ingressClassName` is unset.
const CLASS_ANNOTATION: &str = "kubernetes.io/ingress.class";

/// The port of a service backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendPort {
    /// `port.number`.
    Number(u16),
    /// `port.name`, a named port of the Service.
    Name(String),
    /// Neither is set.
    Unset,
}

impl fmt::Display for BackendPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendPort::Number(n) => write!(f, "{n}"),
            BackendPort::Name(name) => f.write_str(name),
            BackendPort::Unset => Ok(()),
        }
    }
}

/// Where an Ingress rule sends its traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngressBackend {
    /// `service.name` and `service.port`.
    Service {
        /// The Service, in the Ingress' namespace.
        name: String,
        /// The Service's port.
        port: BackendPort,
    },
    /// `resource`: a typed object reference in the Ingress' namespace (a storage bucket).
    Resource {
        /// API group, `None` for the core group.
        api_group: Option<String>,
        /// Kind.
        kind: String,
        /// Name.
        name: String,
    },
}

impl IngressBackend {
    /// The Service this backend names, for the link to it.
    pub fn service(&self) -> Option<&str> {
        match self {
            IngressBackend::Service { name, .. } => Some(name),
            IngressBackend::Resource { .. } => None,
        }
    }

    fn from_json(backend: JsonRef<'_>) -> Option<Self> {
        if let Some(service) = backend.get("service").filter(|s| s.is_object()) {
            let port = sub(service, "port");
            return Some(IngressBackend::Service {
                name: str_of(service, "name")?.to_owned(),
                port: match (
                    port.get("number").and_then(JsonRef::as_u64),
                    str_of(port, "name"),
                ) {
                    (Some(n), _) => {
                        u16::try_from(n).map_or(BackendPort::Unset, BackendPort::Number)
                    }
                    (None, Some(name)) => BackendPort::Name(name.to_owned()),
                    (None, None) => BackendPort::Unset,
                },
            });
        }
        let resource = backend.get("resource")?;
        Some(IngressBackend::Resource {
            api_group: str_of(resource, "apiGroup").map(str::to_owned),
            kind: str_of(resource, "kind")?.to_owned(),
            name: str_of(resource, "name")?.to_owned(),
        })
    }
}

impl fmt::Display for IngressBackend {
    /// `web:80`, `web:http`, or `Bucket/assets` for a resource backend.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngressBackend::Service { name, port } => match port {
                BackendPort::Unset => f.write_str(name),
                port => write!(f, "{name}:{port}"),
            },
            IngressBackend::Resource { kind, name, .. } => write!(f, "{kind}/{name}"),
        }
    }
}

/// One `spec.tls` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressTls {
    /// The hosts the certificate covers; empty means every host.
    pub hosts: Vec<String>,
    /// `secretName`, the Secret holding the certificate.
    pub secret: Option<String>,
}

impl IngressTls {
    /// Whether this entry covers `host` (exactly, or by a `*.` wildcard of one label).
    fn covers(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| host_matches(h, host))
    }
}

/// `pattern` matches `host`: equal, or a wildcard `*.example.com` against exactly one more label.
fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern.eq_ignore_ascii_case(host) {
        return true;
    }
    let (Some(suffix), Some((_, rest))) = (pattern.strip_prefix("*."), host.split_once('.')) else {
        return false;
    };
    !host.starts_with('.') && rest.eq_ignore_ascii_case(suffix)
}

/// One row of the rules table: a host and a path of an Ingress and where they go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressRuleRow {
    /// The rule's host; `None` matches every host.
    pub host: Option<String>,
    /// The path; `None` for a rule that has no HTTP paths at all.
    pub path: Option<String>,
    /// `Exact`, `Prefix` or `ImplementationSpecific`.
    pub path_type: Option<String>,
    /// The backend; `None` for a rule without paths.
    pub backend: Option<IngressBackend>,
    /// The Secret of the TLS entry covering the host, when there is one.
    pub tls_secret: Option<String>,
}

impl IngressRuleRow {
    /// `host/path → service:port`, the one-line form of the table cell.
    pub fn summary(&self) -> String {
        let host = self.host.as_deref().unwrap_or("*");
        let mut text = format!("{host}{}", self.path.as_deref().unwrap_or_default());
        if let Some(backend) = &self.backend {
            text.push_str(" → ");
            text.push_str(&backend.to_string());
        }
        text
    }
}

/// An Ingress: class, addresses, rules, TLS and default backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressSummary {
    /// `spec.ingressClassName`, else the legacy annotation.
    pub class: Option<Arc<str>>,
    /// `status.loadBalancer.ingress[]`: the IP, else the hostname, of each address.
    pub load_balancers: Vec<String>,
    /// The rules, one row per host and path.
    pub rules: Vec<IngressRuleRow>,
    /// `spec.defaultBackend`.
    pub default_backend: Option<IngressBackend>,
    /// `spec.tls`.
    pub tls: Vec<IngressTls>,
}

impl IngressSummary {
    /// The summary of a `networking.k8s.io` Ingress.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Ingress", &[("networking.k8s.io", "Ingress")])?;
        let spec = sub(res.json(), "spec");
        let class = str_of(spec, "ingressClassName")
            .map(Arc::from)
            .or_else(|| res.meta.annotations.get(CLASS_ANNOTATION).cloned());
        let tls: Vec<IngressTls> = arr_of(spec, "tls")
            .iter()
            .map(|entry| IngressTls {
                hosts: arr_of(entry, "hosts")
                    .iter()
                    .filter_map(JsonRef::as_str)
                    .map(str::to_owned)
                    .collect(),
                secret: str_of(entry, "secretName").map(str::to_owned),
            })
            .collect();
        let secret_for = |host: Option<&str>| -> Option<String> {
            let covering = match host {
                Some(host) => tls.iter().find(|entry| entry.covers(host)),
                None => None,
            };
            // An entry without hosts is the default certificate of the Ingress.
            covering
                .or_else(|| tls.iter().find(|entry| entry.hosts.is_empty()))
                .and_then(|entry| entry.secret.clone())
        };
        let mut rules = Vec::new();
        for rule in arr_of(spec, "rules") {
            let host = str_of(rule, "host");
            let tls_secret = secret_for(host);
            let paths = arr_of(sub(rule, "http"), "paths");
            if paths.is_empty() {
                rules.push(IngressRuleRow {
                    host: host.map(str::to_owned),
                    path: None,
                    path_type: None,
                    backend: None,
                    tls_secret: tls_secret.clone(),
                });
            }
            for path in paths {
                rules.push(IngressRuleRow {
                    host: host.map(str::to_owned),
                    path: Some(str_of(path, "path").unwrap_or("/").to_owned()),
                    path_type: str_of(path, "pathType").map(str::to_owned),
                    backend: IngressBackend::from_json(sub(path, "backend")),
                    tls_secret: tls_secret.clone(),
                });
            }
        }
        let load_balancers = arr_of(sub(sub(res.json(), "status"), "loadBalancer"), "ingress")
            .iter()
            .filter_map(|a| str_of(a, "ip").or_else(|| str_of(a, "hostname")))
            .map(str::to_owned)
            .collect();
        Ok(Self {
            class,
            load_balancers,
            rules,
            default_backend: IngressBackend::from_json(sub(spec, "defaultBackend")),
            tls,
        })
    }

    /// The hosts of the rules, each once, in order; `*` stands for a rule without a host.
    pub fn hosts(&self) -> Vec<&str> {
        let mut hosts: Vec<&str> = Vec::new();
        for row in &self.rules {
            let host = row.host.as_deref().unwrap_or("*");
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        hosts
    }

    /// The `Rules` cell: each row's [summary](IngressRuleRow::summary), comma-separated, the
    /// default backend last.
    pub fn rules_text(&self) -> String {
        let mut parts: Vec<String> = self.rules.iter().map(IngressRuleRow::summary).collect();
        if let Some(backend) = &self.default_backend {
            parts.push(format!("(default) → {backend}"));
        }
        parts.join(", ")
    }

    /// The Services the Ingress sends traffic to, each once, in order.
    pub fn services(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        let backends = self
            .rules
            .iter()
            .filter_map(|r| r.backend.as_ref())
            .chain(self.default_backend.as_ref());
        for name in backends.filter_map(IngressBackend::service) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }
}
