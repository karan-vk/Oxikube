//! [`ProviderStatus`]: what the app knows about one cluster's metrics provider.

use jiff::Timestamp;
use oxikube_ports::{DetectedProvider, DetectionState, MetricsMode, Probe, PrometheusOverride};

/// Where a cluster's provider stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderState {
    /// The settings turn history metrics off for the cluster (`enabled: false` or `provider:
    /// off`). Nothing is probed.
    Disabled,
    /// The cluster is not connected, so there is nothing to probe yet.
    WaitingForCluster,
    /// Detection (or the check of a configured endpoint) is running, found something, found
    /// nothing, or failed.
    Detection(DetectionState),
}

/// One cluster's provider status: the settings' mode, the state, and the services probed.
///
/// A value: the service hands out clones (`Arc`s of one) and replaces it as things change, so a
/// view reads it without a lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    /// What the settings ask for (`Off` when disabled).
    pub mode: MetricsMode,
    /// Where it stands.
    pub state: ProviderState,
    /// The services detection looked at, in catalogue order. Empty for a configured endpoint.
    pub probes: Vec<Probe>,
    /// When detection last finished, if it has.
    pub checked_at: Option<Timestamp>,
    /// The cluster's `prometheus` settings in effect, for the settings page to show next to the
    /// state.
    pub settings: PrometheusOverride,
}

impl ProviderStatus {
    /// A status in `state`, with no probes.
    pub fn new(mode: MetricsMode, state: ProviderState) -> Self {
        Self {
            mode,
            state,
            probes: Vec::new(),
            checked_at: None,
            settings: PrometheusOverride::default(),
        }
    }

    /// The same status for the settings `settings`.
    #[must_use]
    pub fn with_settings(mut self, settings: PrometheusOverride) -> Self {
        self.settings = settings;
        self
    }

    /// The provider that answered, if one did.
    pub fn provider(&self) -> Option<&DetectedProvider> {
        match &self.state {
            ProviderState::Detection(DetectionState::Found(provider)) => Some(provider),
            _ => None,
        }
    }

    /// Whether a run is in progress.
    pub fn is_detecting(&self) -> bool {
        matches!(
            self.state,
            ProviderState::Detection(DetectionState::Detecting)
        )
    }

    /// Why there are no history metrics, as the text after `metrics unavailable: `. `None` when
    /// a provider answered or detection is still running.
    pub fn unavailable_reason(&self) -> Option<String> {
        match &self.state {
            ProviderState::Disabled => Some("turned off in this cluster's settings".to_owned()),
            ProviderState::WaitingForCluster => Some("the cluster is not connected".to_owned()),
            ProviderState::Detection(DetectionState::Detecting)
            | ProviderState::Detection(DetectionState::Found(_)) => None,
            ProviderState::Detection(DetectionState::NotFound) => Some(not_found(&self.probes)),
            ProviderState::Detection(DetectionState::Failed(failure)) => Some(failure.reason()),
        }
    }

    /// `metrics unavailable: <reason>`, or `None` when metrics are (or are about to be)
    /// available.
    pub fn unavailable_text(&self) -> Option<String> {
        self.unavailable_reason()
            .map(|reason| format!("metrics unavailable: {reason}"))
    }

    /// One line for a status row.
    pub fn headline(&self) -> String {
        match &self.state {
            ProviderState::Detection(DetectionState::Found(provider)) => {
                format!("Using {}", provider.summary())
            }
            ProviderState::Detection(DetectionState::Detecting) => match self.mode {
                MetricsMode::Manual => "Checking the configured endpoint…".to_owned(),
                _ => "Looking for a Prometheus-compatible service…".to_owned(),
            },
            _ => self.unavailable_text().unwrap_or_default(),
        }
    }
}

fn not_found(probes: &[Probe]) -> String {
    if probes.is_empty() {
        return "no Prometheus-compatible service was found; configure one by hand".to_owned();
    }
    format!(
        "no Prometheus-compatible service was found (looked for {} known services); configure \
         one by hand",
        probes.len()
    )
}

#[cfg(test)]
mod tests {
    use oxikube_domain::MetricsProviderKind;
    use oxikube_ports::{DetectionFailure, ProbeOutcome, ProviderTarget, ServicePort, ServiceRef};

    use super::*;

    fn found() -> ProviderStatus {
        ProviderStatus::new(
            MetricsMode::Auto,
            ProviderState::Detection(DetectionState::Found(DetectedProvider {
                kind: MetricsProviderKind::KubePrometheus,
                target: ProviderTarget::Service(ServiceRef::new(
                    "monitoring",
                    "prometheus-operated",
                    ServicePort::Name("web".into()),
                )),
                path_prefix: None,
            })),
        )
    }

    #[test]
    fn a_found_provider_has_no_unavailable_text() {
        let status = found();
        assert!(status.provider().is_some());
        assert_eq!(status.unavailable_text(), None);
        assert_eq!(
            status.headline(),
            "Using kube-prometheus at monitoring/prometheus-operated:web"
        );
    }

    #[test]
    fn every_other_state_explains_itself_after_metrics_unavailable() {
        let cases = [
            (ProviderState::Disabled, "turned off"),
            (ProviderState::WaitingForCluster, "not connected"),
            (
                ProviderState::Detection(DetectionState::NotFound),
                "no Prometheus-compatible service",
            ),
            (
                ProviderState::Detection(DetectionState::Failed(DetectionFailure::Forbidden(
                    "cannot list services in monitoring".into(),
                ))),
                "configure the endpoint by hand",
            ),
        ];
        for (state, needle) in cases {
            let status = ProviderStatus::new(MetricsMode::Auto, state);
            let text = status.unavailable_text().unwrap();
            assert!(text.starts_with("metrics unavailable: "), "{text}");
            assert!(text.contains(needle), "{text}");
            assert!(status.provider().is_none());
        }
    }

    #[test]
    fn detecting_is_neither_available_nor_unavailable() {
        let status = ProviderStatus::new(
            MetricsMode::Auto,
            ProviderState::Detection(DetectionState::Detecting),
        );
        assert!(status.is_detecting());
        assert_eq!(status.unavailable_text(), None);
        assert!(status.headline().starts_with("Looking for"));
    }

    #[test]
    fn not_found_counts_the_services_looked_for() {
        let mut status = ProviderStatus::new(
            MetricsMode::Auto,
            ProviderState::Detection(DetectionState::NotFound),
        );
        status.probes = vec![Probe {
            namespace: "monitoring".into(),
            service: "prometheus-operated".into(),
            kind: MetricsProviderKind::KubePrometheus,
            outcome: ProbeOutcome::NotPresent,
        }];
        assert!(
            status
                .unavailable_text()
                .unwrap()
                .contains("looked for 1 known")
        );
    }
}
