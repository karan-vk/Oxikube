//! Metrics provider detection: [`ProviderDetectorPort`] and the result types (E13-S05).
//!
//! A cluster's PromQL history comes from a Prometheus-compatible service somewhere in it
//! (kube-prometheus, the Lens stack, VictoriaMetrics, Mimir, OpenShift's Thanos querier). The
//! detector lists the Services of the namespaces those stacks use, matches them against a
//! catalogue, confirms a match with a cheap probe query and classifies the stack as a
//! [`MetricsProviderKind`], which the query catalogue (E13-S06) uses to pick label names.
//!
//! Absence is a visible state, never a swallowed error (ADR 0011): [`DetectionState`] says
//! whether a provider was found, none exists, or the attempt failed and why, and
//! [`DetectionResult::probes`] lists every service looked at so the settings page can explain a
//! missing stack.
//!
//! # Adapter
//!
//! Implemented by `oxikube_prometheus` (`ProviderDetector`) over a cluster's
//! [`ResourceReader`] (read-only `list services`) and [`ServiceProxyPort`] (the probe query).

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::MetricsProviderKind;

use crate::promql::{PromEndpoint, PromqlError};
use crate::resource::ResourceReader;
use crate::service_proxy::{ServiceProxyPort, ServiceRef};

/// Where a provider's API is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTarget {
    /// A Service reached through the API server's service proxy (what detection finds).
    Service(ServiceRef),
    /// A URL the user configured, reached directly.
    Url(String),
}

impl fmt::Display for ProviderTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderTarget::Service(service) => write!(f, "{service}"),
            ProviderTarget::Url(url) => f.write_str(url),
        }
    }
}

/// A provider that answered a probe query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProvider {
    /// Which stack it is.
    pub kind: MetricsProviderKind,
    /// Where it is.
    pub target: ProviderTarget,
    /// A path inserted before `/api/v1/...` (Mimir's `/prometheus`, VictoriaMetrics' cluster
    /// select path). `None` when the API is at the root.
    pub path_prefix: Option<String>,
}

impl DetectedProvider {
    /// The endpoint queries go to: no auth (a service proxy carries the connection's own
    /// credentials), default TLS, timeout and size limit.
    pub fn endpoint(&self) -> PromEndpoint {
        let mut endpoint = match &self.target {
            ProviderTarget::Service(service) => PromEndpoint::service(service.clone()),
            ProviderTarget::Url(url) => PromEndpoint::new(url.clone()),
        };
        endpoint.path_prefix = self.path_prefix.clone();
        endpoint
    }

    /// `kind at target`, for one line of status text.
    pub fn summary(&self) -> String {
        match &self.path_prefix {
            Some(prefix) => format!("{} at {}{prefix}", self.kind, self.target),
            None => format!("{} at {}", self.kind, self.target),
        }
    }
}

/// Why detection could not conclude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionFailure {
    /// The user may not list Services (or use the service proxy) here: the cluster may well have
    /// a Prometheus that cannot be looked for. Suggest configuring it by hand.
    Forbidden(String),
    /// A Service matched a known name, or a configured endpoint was given, but the probe query
    /// to it failed (proxy blocked, wrong port, not a Prometheus API, TLS or auth in front of it).
    ProbeFailed {
        /// The stack the name (or the configuration) suggested.
        kind: MetricsProviderKind,
        /// What was probed.
        target: ProviderTarget,
        /// What went wrong, safe to show.
        reason: String,
    },
    /// Anything else: the API server did not answer, a list failed.
    Other(String),
}

impl DetectionFailure {
    /// The reason as one line: what failed and, where it helps, what to do.
    pub fn reason(&self) -> String {
        match self {
            DetectionFailure::Forbidden(detail) => format!(
                "not allowed to look for a metrics service ({detail}); configure the endpoint \
                 by hand instead"
            ),
            DetectionFailure::ProbeFailed {
                kind,
                target,
                reason,
            } => format!(
                "{target} ({kind}) did not answer a query: {reason}; check the address, \
                 credentials and path prefix, or configure the endpoint by hand"
            ),
            DetectionFailure::Other(detail) => detail.clone(),
        }
    }

    /// `true` when the failure is about permissions.
    pub fn is_forbidden(&self) -> bool {
        matches!(self, DetectionFailure::Forbidden(_))
    }
}

/// The outcome of detecting (or checking a configured endpoint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionState {
    /// Detection is running.
    Detecting,
    /// A provider answered.
    Found(DetectedProvider),
    /// Every candidate namespace was listed and nothing matched.
    NotFound,
    /// Detection could not conclude.
    Failed(DetectionFailure),
}

/// What happened to one catalogue entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The Service exists and answered the probe query.
    Answered,
    /// No Service of that name exists in the namespace.
    NotPresent,
    /// The Service exists but exposes none of the ports the catalogue expects.
    NoUsablePort,
    /// The Service exists but the probe query failed.
    Failed(String),
    /// The namespace could not be listed (RBAC or a failed list), so the entry was not checked.
    Skipped(String),
    /// A better match answered first, so this one was not probed.
    NotNeeded,
}

impl ProbeOutcome {
    /// A short phrase for the settings page.
    pub fn text(&self) -> String {
        match self {
            ProbeOutcome::Answered => "answered".to_owned(),
            ProbeOutcome::NotPresent => "not found".to_owned(),
            ProbeOutcome::NoUsablePort => "found, but no usable port".to_owned(),
            ProbeOutcome::Failed(reason) => format!("found, but: {reason}"),
            ProbeOutcome::Skipped(reason) => format!("not checked: {reason}"),
            ProbeOutcome::NotNeeded => "not needed".to_owned(),
        }
    }
}

/// One service looked for, and what came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The namespace looked in.
    pub namespace: String,
    /// The service name looked for (a name ending in `*` stands for a prefix).
    pub service: String,
    /// The stack the catalogue says it belongs to.
    pub kind: MetricsProviderKind,
    /// What happened.
    pub outcome: ProbeOutcome,
}

impl Probe {
    /// `namespace/service`.
    pub fn label(&self) -> String {
        format!("{}/{}", self.namespace, self.service)
    }
}

/// A detection run: the verdict, and every service looked at on the way to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionResult {
    /// The verdict.
    pub state: DetectionState,
    /// The services probed, in catalogue order.
    pub probes: Vec<Probe>,
    /// When the run finished, if it has.
    pub finished_at: Option<Timestamp>,
}

impl DetectionResult {
    /// A result with `state` and no probes.
    pub fn of(state: DetectionState) -> Self {
        Self {
            state,
            probes: Vec::new(),
            finished_at: None,
        }
    }

    /// A run in progress.
    pub fn detecting() -> Self {
        Self::of(DetectionState::Detecting)
    }
}

/// Finds and checks Prometheus-compatible services.
///
/// # Effects
///
/// Read-only: lists Services and sends a probe query through the service proxy. Nothing is
/// installed or changed in the cluster.
///
/// # Errors
///
/// None are returned: a failure is a [`DetectionState::Failed`] (detection) or a
/// [`PromqlError`] (checking a configured endpoint), so it is shown, never swallowed. Dropping
/// the future cancels the run.
#[async_trait]
pub trait ProviderDetectorPort: Send + Sync {
    /// Looks for a provider in the cluster `resources` and `proxy` belong to.
    async fn detect(
        &self,
        resources: Arc<dyn ResourceReader>,
        proxy: Arc<dyn ServiceProxyPort>,
    ) -> DetectionResult;

    /// Checks that `endpoint` answers a probe query: through `proxy` for a Service endpoint,
    /// directly for a URL.
    async fn check(
        &self,
        proxy: Arc<dyn ServiceProxyPort>,
        endpoint: &PromEndpoint,
    ) -> Result<(), PromqlError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_proxy::ServicePort;

    fn service() -> ServiceRef {
        ServiceRef::new(
            "monitoring",
            "prometheus-operated",
            ServicePort::Number(9090),
        )
    }

    #[test]
    fn a_detected_service_becomes_a_proxy_endpoint() {
        let provider = DetectedProvider {
            kind: MetricsProviderKind::Mimir,
            target: ProviderTarget::Service(service()),
            path_prefix: Some("/prometheus".into()),
        };
        let endpoint = provider.endpoint();
        assert_eq!(endpoint.service, Some(service()));
        assert_eq!(endpoint.path_prefix.as_deref(), Some("/prometheus"));
        assert_eq!(
            provider.summary(),
            "Mimir at monitoring/prometheus-operated:9090/prometheus"
        );
    }

    #[test]
    fn a_configured_url_becomes_a_direct_endpoint() {
        let provider = DetectedProvider {
            kind: MetricsProviderKind::VictoriaMetrics,
            target: ProviderTarget::Url("https://vm.example.com".into()),
            path_prefix: None,
        };
        let endpoint = provider.endpoint();
        assert_eq!(endpoint.service, None);
        assert_eq!(endpoint.base_url, "https://vm.example.com");
    }

    #[test]
    fn failures_say_what_to_do() {
        let forbidden = DetectionFailure::Forbidden("services is forbidden".into());
        assert!(forbidden.is_forbidden());
        assert!(
            forbidden
                .reason()
                .contains("configure the endpoint by hand")
        );
        let probe = DetectionFailure::ProbeFailed {
            kind: MetricsProviderKind::KubePrometheus,
            target: ProviderTarget::Service(service()),
            reason: "unreachable".into(),
        };
        let text = probe.reason();
        assert!(
            text.contains("monitoring/prometheus-operated:9090"),
            "{text}"
        );
        assert!(text.contains("unreachable"), "{text}");
    }
}
