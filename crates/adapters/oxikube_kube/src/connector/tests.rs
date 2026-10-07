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

fn one_context_kubeconfig() -> Kubeconfig {
    // Building the client reads nothing from the network; the server address is never dialled.
    Kubeconfig::from_yaml(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: { server: \"https://127.0.0.1:1\" }\n\
         users:\n- name: u\n  user: { token: \"not-a-real-token\" }\n\
         contexts:\n- name: only\n  context: { cluster: c, user: u }\ncurrent-context: only\n",
    )
    .unwrap()
}

#[tokio::test]
async fn a_connection_without_a_describe_factory_cannot_describe() {
    let connector = KubeConnector::new(
        one_context_kubeconfig(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let connection = connector
        .connect(request("only", ExecInteractivity::Never))
        .await
        .unwrap();
    let target = oxikube_domain::ids::ResourceRef::namespaced(
        ClusterId::new("test", &ContextName::new("only")),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "default",
        "p",
    );
    let error = connection
        .ports
        .describe
        .describe(&target)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn the_describe_factory_gets_the_connections_context_and_its_port_is_used() {
    use std::sync::Mutex;

    struct Canned;
    #[async_trait::async_trait]
    impl oxikube_ports::DescribePort for Canned {
        async fn describe(
            &self,
            _: &oxikube_domain::ids::ResourceRef,
        ) -> oxikube_domain::OxiResult<oxikube_ports::DescribeOutput> {
            Ok(oxikube_ports::DescribeOutput {
                text: "canned".into(),
                source: oxikube_ports::DescribeSource::Native,
            })
        }
    }

    let connector = KubeConnector::new(
        one_context_kubeconfig(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    connector.set_describe_factory(Arc::new(move |connection: DescribeConnection| {
        log.lock().unwrap().push(connection.context.to_string());
        Arc::new(Canned)
    }));
    let connection = connector
        .connect(request("only", ExecInteractivity::Never))
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["only"]);
    let target = oxikube_domain::ids::ResourceRef::namespaced(
        ClusterId::new("test", &ContextName::new("only")),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "default",
        "p",
    );
    let output = connection.ports.describe.describe(&target).await.unwrap();
    assert_eq!(output.text, "canned");
}

#[tokio::test]
async fn a_connections_feeds_go_through_its_watch_budget_set_per_cluster() {
    let connector = empty_connector();
    connector.replace_loaded(loaded_with("budgeted"));
    let req = request("budgeted", ExecInteractivity::Never);
    let cluster = req.cluster.clone();
    let budgeted = cluster.clone();
    connector.set_budget_for(Arc::new(move |cluster: &ClusterId| BudgetConfig {
        // No feed fits, so a watch that reaches the budget is refused before any request.
        max_feeds: if *cluster == budgeted { 0 } else { 64 },
        ..BudgetConfig::default()
    }));
    let connection = connector
        .connect(req)
        .await
        .expect("connects without a request");

    let registry = connector.feeds(&cluster).expect("a live budget");
    assert_eq!(registry.config().max_feeds, 0, "from set_budget_for");
    assert_eq!(connector.registries().len(), 1);
    let gvk = oxikube_domain::ids::Gvk::new("", "v1", "Pod");
    let watch = connection
        .ports
        .resources
        .watch(
            &gvk,
            Some("default"),
            &oxikube_ports::WatchOptions::default(),
        )
        .await;
    assert_eq!(
        watch.err().map(|e| e.kind()),
        Some(ErrorKind::BudgetExceeded),
        "the port's watch is a feed of the budget"
    );
    let table = connection
        .ports
        .tables
        .table_feed(&gvk, None, &oxikube_ports::TableOptions::default())
        .await;
    assert_eq!(
        table.err().map(|e| e.kind()),
        Some(ErrorKind::BudgetExceeded)
    );
    assert_eq!(registry.stats().refused, 2);

    drop(connection);
    assert!(
        connector.registries().is_empty(),
        "gone with the connection"
    );
}
