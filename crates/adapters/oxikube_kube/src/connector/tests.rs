use std::sync::Arc;

use kube::config::Kubeconfig;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterConnectorPort, ConnectRequest, ExecInteractivity, HealthReporter, HealthSignal,
};

use super::connection::signal;
use super::*;
use crate::health::HealthEvent;

struct NoReports;

impl HealthReporter for NoReports {
    fn report(&self, _signal: HealthSignal) {}
}

fn empty_connector() -> KubeConnector {
    let kubeconfig: Kubeconfig = serde_json::from_str("{}").unwrap();
    KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig::default(),
    )
}

fn request(context: &str, exec_interactivity: ExecInteractivity) -> ConnectRequest {
    let context = ContextName::new(context);
    ConnectRequest {
        cluster: ClusterId::new("test", &context),
        context,
        exec_interactivity,
        health: Arc::new(NoReports),
    }
}

#[tokio::test]
async fn a_context_missing_from_the_kubeconfig_is_not_found_and_leaves_no_connection() {
    let connector = empty_connector();
    let req = request("nope", ExecInteractivity::Never);
    let cluster = req.cluster.clone();
    let err = connector.connect(req).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(connector.feeds(&cluster).is_none());
}

#[tokio::test]
async fn one_pool_is_built_per_exec_policy_and_reused() {
    let connector = empty_connector();
    let never = connector.pool(ExecInteractivity::Never);
    assert!(Arc::ptr_eq(
        &never,
        &connector.pool(ExecInteractivity::Never)
    ));
    let always = connector.pool(ExecInteractivity::Always);
    assert!(!Arc::ptr_eq(&never, &always));
    assert_eq!(always.config().exec_policy, ExecInteractivePolicy::Always);
    assert_eq!(never.config().exec_policy, ExecInteractivePolicy::Never);
}

#[test]
fn probe_results_map_onto_health_signals() {
    let healthy = HealthEvent::Healthy {
        server_version: "v1.31.0".into(),
    };
    assert_eq!(signal(&healthy), HealthSignal::Healthy);
    let unhealthy = HealthEvent::Unhealthy {
        error: OxiError::network("connection refused"),
        consecutive_failures: 1,
    };
    assert_eq!(signal(&unhealthy), HealthSignal::Unhealthy);
    let failed = HealthEvent::Failed {
        error: OxiError::network("gone"),
    };
    assert!(matches!(
        signal(&failed),
        HealthSignal::Failed { reason } if reason.contains("gone")
    ));
}
