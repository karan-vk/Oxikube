//! One run of the service: detection in auto mode, a check of the configured endpoint in manual
//! mode.

use std::sync::Arc;

use oxikube_ports::{
    DetectedProvider, DetectionFailure, DetectionResult, DetectionState, MetricsMode, PromEndpoint,
    PrometheusOverride, ProviderDetectorPort, ProviderTarget, ResourceReader, ServiceProxyPort,
};

/// What a run produced: the verdict, and the endpoint to query when it found a provider.
pub(super) struct Found {
    pub(super) result: DetectionResult,
    pub(super) endpoint: Option<PromEndpoint>,
}

/// One run: detection in auto mode, a check of the configured endpoint in manual mode.
pub(super) async fn detect(
    detector: Arc<dyn ProviderDetectorPort>,
    mode: MetricsMode,
    prefs: PrometheusOverride,
    resources: Arc<dyn ResourceReader>,
    proxy: Arc<dyn ServiceProxyPort>,
) -> Found {
    let nothing = |result| Found {
        result,
        endpoint: None,
    };
    match mode {
        MetricsMode::Auto => {
            let result = detector.detect(resources, proxy).await;
            let endpoint = match &result.state {
                DetectionState::Found(provider) => Some(provider.endpoint()),
                _ => None,
            };
            Found { result, endpoint }
        }
        MetricsMode::Manual => {
            let (endpoint, kind) = match prefs.manual_endpoint() {
                Ok(configured) => configured,
                Err(message) => {
                    return nothing(DetectionResult::of(DetectionState::Failed(
                        DetectionFailure::Other(message),
                    )));
                }
            };
            let target = match &endpoint.service {
                Some(service) => ProviderTarget::Service(service.clone()),
                None => ProviderTarget::Url(endpoint.base_url.clone()),
            };
            let (state, endpoint) = match detector.check(proxy, &endpoint).await {
                Ok(()) => (
                    DetectionState::Found(DetectedProvider {
                        kind,
                        target,
                        path_prefix: endpoint.path_prefix.clone(),
                    }),
                    Some(endpoint),
                ),
                Err(error) => (
                    DetectionState::Failed(DetectionFailure::ProbeFailed {
                        kind,
                        target,
                        reason: error.to_string(),
                    }),
                    None,
                ),
            };
            Found {
                result: DetectionResult {
                    state,
                    probes: Vec::new(),
                    finished_at: Some(jiff::Timestamp::now()),
                },
                endpoint,
            }
        }
        // Not run: the caller records `Disabled`.
        MetricsMode::Off => nothing(DetectionResult::of(DetectionState::NotFound)),
    }
}
