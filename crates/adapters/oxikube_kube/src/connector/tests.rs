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

/// A loader result holding one token-auth context `name` on an address nothing listens on.
fn loaded_with(name: &str) -> Arc<LoadedKubeconfig> {
    let merged: Kubeconfig = serde_json::from_value(serde_json::json!({
        "clusters": [{ "name": "c", "cluster": { "server": "https://127.0.0.1:1" } }],
        "users": [{ "name": "u", "user": { "token": "not-a-real-token" } }],
        "contexts": [{ "name": name, "context": { "cluster": "c", "user": "u" } }],
    }))
    .unwrap();
    Arc::new(LoadedKubeconfig {
        merged,
        sources: Vec::new(),
        origins: Default::default(),
        diagnostics: Vec::new(),
    })
}

#[tokio::test]
async fn a_reloaded_kubeconfig_reaches_pools_built_before_and_after_it() {
    let connector = empty_connector();
    let context = ContextName::new("added");
    let before = connector.pool(ExecInteractivity::Never);
    assert_eq!(
        before.get(&context).await.err().map(|e| e.kind()),
        Some(ErrorKind::NotFound)
    );

    let dropped = connector.replace_loaded(loaded_with("added"));
    assert!(dropped.is_empty(), "nothing was pooled: {dropped:?}");
    assert!(
        before.get(&context).await.is_ok(),
        "the existing pool sees it"
    );
    let after = connector.pool(ExecInteractivity::Always);
    assert!(after.get(&context).await.is_ok(), "a later pool sees it");

    // The context goes away again: its pooled clients are dropped, once per name.
    let dropped = connector.replace_loaded(loaded_with("other"));
    assert_eq!(dropped, vec![context.clone()]);
    assert_eq!(
        before.get(&context).await.err().map(|e| e.kind()),
        Some(ErrorKind::NotFound)
    );
}
