//! The store's budget hook over a registry, and settings reaching live connections.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kube::config::Kubeconfig;
use oxikube_app::store::{FeedKey, FeedPriority, FeedScope, LabelSelector};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ContextName, Gvk};
use oxikube_kube::{ByteCounter, ConnectorConfig, FeedSource, FeedStream, PoolConfig};
use oxikube_ports::{
    ClusterConnectorPort as _, ConnectRequest, ExecInteractivity, HealthReporter, HealthSignal,
};

use super::*;

/// Feeds that never deliver anything.
struct Silent;

#[async_trait]
impl FeedSource for Silent {
    async fn open(&self, _: &FeedRequest, _: ByteCounter) -> OxiResult<FeedStream> {
        Ok(FeedStream::Resources(Box::pin(futures::stream::pending())))
    }
}

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("test", &ContextName::new(name))
}

fn registry(config: BudgetConfig) -> FeedRegistry {
    FeedRegistry::new(cluster("kind"), Arc::new(Silent), config)
}

fn store_request(kind: FeedKind, namespace: &str) -> StoreRequest {
    StoreRequest {
        key: FeedKey::new(
            Gvk::new("", "v1", "Pod"),
            FeedScope::Namespace(namespace.into()),
        ),
        kind,
        priority: FeedPriority::High,
    }
}

/// Opens an owned feed the way `BudgetedResources` names a store feed.
async fn open(registry: &FeedRegistry, request: &StoreRequest) -> FeedStream {
    registry
        .open_owned(registry_request(request), |_| async {
            Silent
                .open(&registry_request(request), ByteCounter::default())
                .await
        })
        .await
        .expect("admitted")
}

#[test]
fn the_default_setting_is_the_adapters_default_budget() {
    assert_eq!(
        budget_config(&WatchBudgetPrefs::default()),
        BudgetConfig::default()
    );
}

#[test]
fn a_store_feed_has_the_name_its_port_call_gets() {
    let mut request = store_request(FeedKind::Table, "web");
    request.key = request
        .key
        .with_selector(Some(LabelSelector::parse("app=web").unwrap()));
    let named = registry_request(&request);
    assert_eq!(named.namespace.as_deref(), Some("web"));
    assert_eq!(named.variant, FeedVariant::Table);
    assert_eq!(named.label_selector.as_deref(), Some("app=web"));
}

#[tokio::test]
async fn the_hook_refuses_with_the_registrys_reason_and_frees_a_released_feed() {
    let registry = registry(BudgetConfig {
        max_feeds: 1,
        idle_grace: Duration::from_secs(7),
        ..BudgetConfig::default()
    });
    let hook = RegistryBudget::new(registry.clone());
    let first = store_request(FeedKind::Full, "a");
    assert_eq!(hook.admit(&first, 0), Admission::Granted);
    let _feed = open(&registry, &first).await;

    let second = store_request(FeedKind::Full, "b");
    assert!(matches!(
        hook.admit(&second, 1),
        Admission::Refused(reason) if reason.contains("1 of 1 feeds")
    ));
    // The store evicts its idle feed: released at once, before its task drops the stream.
    hook.released(&first);
    assert_eq!(hook.admit(&second, 0), Admission::Granted);
    assert_eq!(hook.idle_grace(), Some(Duration::from_secs(7)));
    hook.refused(&second);
    assert_eq!(
        registry.stats().refused,
        1,
        "the store's final refusal is counted"
    );
}

#[tokio::test]
async fn a_full_kind_past_the_threshold_is_degraded_and_a_table_is_not() {
    let registry = registry(BudgetConfig {
        metadata_above: 0,
        ..BudgetConfig::default()
    });
    let hook = RegistryBudget::new(registry.clone());
    assert_eq!(
        hook.admit(&store_request(FeedKind::Full, "a"), 0),
        Admission::Degraded(FeedKind::Metadata)
    );
    assert_eq!(registry.stats().degraded, 1);
    assert_eq!(
        hook.admit(&store_request(FeedKind::Table, "a"), 0),
        Admission::Granted
    );
}

struct NoReports;

impl HealthReporter for NoReports {
    fn report(&self, _: HealthSignal) {}
}

/// A connector over one token context on an address nothing listens on (connects need no
/// request).
fn connector(contexts: &[&str]) -> KubeConnector {
    let contexts: Vec<_> = contexts
        .iter()
        .map(|name| serde_json::json!({ "name": name, "context": { "cluster": "c", "user": "u" } }))
        .collect();
    let kubeconfig: Kubeconfig = serde_json::from_value(serde_json::json!({
        "clusters": [{ "name": "c", "cluster": { "server": "https://127.0.0.1:1" } }],
        "users": [{ "name": "u", "user": { "token": "not-a-real-token" } }],
        "contexts": contexts,
    }))
    .unwrap();
    KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig::default(),
    )
}

fn request(name: &str) -> ConnectRequest {
    let context = ContextName::new(name);
    ConnectRequest {
        cluster: ClusterId::new("test", &context),
        context,
        exec_interactivity: ExecInteractivity::Never,
        health: Arc::new(NoReports),
    }
}

fn prefs_with(max_feeds: usize) -> ClusterPrefs {
    ClusterPrefs {
        watch_budget: WatchBudgetPrefs {
            max_feeds,
            ..WatchBudgetPrefs::default()
        },
        ..ClusterPrefs::default()
    }
}

#[tokio::test]
async fn settings_reach_new_and_live_connections_per_cluster() {
    let kube = connector(&["prod", "lab"]);
    let budgets = WatchBudgets::new(&kube);
    budgets
        .apply(ClusterPrefsTable::new(prefs_with(64)).with_cluster(cluster("prod"), prefs_with(8)));
    let _prod = kube.connect(request("prod")).await.unwrap();
    let _lab = kube.connect(request("lab")).await.unwrap();
    let max = |name: &str| budgets.registry(&cluster(name)).unwrap().config().max_feeds;
    assert_eq!((max("prod"), max("lab")), (8, 64), "at connect");

    // A hot reload: the live connections change without a reconnect.
    budgets
        .apply(ClusterPrefsTable::new(prefs_with(32)).with_cluster(cluster("prod"), prefs_with(2)));
    assert_eq!((max("prod"), max("lab")), (2, 32));
    let stats = budgets.stats();
    assert_eq!(stats.len(), 2);
    assert!(report_line("prod", &stats[0]).contains("feeds 0/"));

    // The stores of a connected cluster ask its budget; the others are unlimited.
    let options = budgets.store_options();
    let hook = options(&cluster("prod"), &ClusterPrefs::default()).budget;
    assert_eq!(hook.idle_grace(), Some(Duration::from_secs(30)));
    let other = options(&cluster("gone"), &ClusterPrefs::default()).budget;
    assert_eq!(other.idle_grace(), None);
}

#[test]
fn disabled_budgets_have_no_counters() {
    let budgets = WatchBudgets::disabled();
    budgets.apply(ClusterPrefsTable::default());
    assert!(budgets.stats().is_empty());
    assert!(budgets.registry(&cluster("kind")).is_none());
}
