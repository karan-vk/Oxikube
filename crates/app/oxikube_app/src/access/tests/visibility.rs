//! The sidebar's visibility hook over cached rules reviews.

use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{FakeClusterConnectorPort, FakeClusterSourcePort};

use crate::access::{AccessRegistry, AccessVisibility};
use crate::kinds::KindVisibility;
use crate::session::ClusterSessionManager;

use super::*;

struct Rig {
    cluster: ClusterId,
    visibility: AccessVisibility,
    port: Arc<FakeAccessReviewPort>,
    clock: Arc<FakeClockPort>,
}

async fn rig() -> Rig {
    let name = ContextName::new("a");
    let cluster = ClusterId::new("/home/me/.kube/config", &name);
    let ctx = ClusterContext::new(cluster.clone(), name, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([ctx.clone()]));
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    manager.open(&ctx, Default::default());
    manager.connect(&cluster).await.expect("connect");
    let port = connector.ports_for(&cluster).access;
    let visibility = AccessVisibility::new(manager, AccessRegistry::new(clock.clone()));
    Rig {
        cluster,
        visibility,
        port,
        clock,
    }
}

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn secret() -> Gvk {
    Gvk::new("", "v1", "Secret")
}

fn listable(rig: &Rig, gvk: &Gvk, plural: &str, namespace: &str) -> Option<bool> {
    rig.visibility
        .is_listable_resource(&rig.cluster, gvk, Some(plural), Some(namespace))
}

#[tokio::test]
async fn nothing_is_hidden_before_the_review_arrives() {
    let rig = rig().await;
    assert_eq!(listable(&rig, &secret(), "secrets", "dev"), None);
    assert_eq!(
        rig.visibility
            .is_listable_resource(&rig.cluster, &pod(), None, Some("dev")),
        None,
        "no plural yet (discovery has not answered): unknown, so shown"
    );
}

#[tokio::test]
async fn a_kind_the_namespace_rules_deny_is_not_listable_and_an_allowed_one_is() {
    let rig = rig().await;
    rig.port.set_namespace_rules("dev", reads(&["pods"]));
    rig.visibility
        .refresh(&rig.cluster, &NamespaceSelection::from_names(["dev"]))
        .await;

    assert_eq!(listable(&rig, &pod(), "pods", "dev"), Some(true));
    assert_eq!(listable(&rig, &secret(), "secrets", "dev"), Some(false));
    assert_eq!(
        listable(&rig, &pod(), "pods", "prod"),
        None,
        "a namespace nobody reviewed is unknown, not denied"
    );
}

#[tokio::test]
async fn the_last_answer_survives_the_ttl() {
    let rig = rig().await;
    rig.port.set_namespace_rules("dev", reads(&["pods"]));
    rig.visibility
        .refresh(&rig.cluster, &NamespaceSelection::from_names(["dev"]))
        .await;
    rig.clock.advance(PAST_TTL);

    assert_eq!(
        listable(&rig, &secret(), "secrets", "dev"),
        Some(false),
        "an aged review must not make hidden entries reappear"
    );
}

#[tokio::test]
async fn a_failed_review_hides_nothing() {
    let rig = rig().await;
    // The cluster-wide ask and the namespace's both fail.
    for _ in 0..2 {
        rig.port
            .script()
            .rules
            .push_err(oxikube_domain::OxiError::network("apiserver unreachable"));
    }
    rig.visibility
        .refresh(&rig.cluster, &NamespaceSelection::from_names(["dev"]))
        .await;

    assert_eq!(listable(&rig, &secret(), "secrets", "dev"), None);
}

#[tokio::test]
async fn cluster_scoped_kinds_follow_the_cluster_wide_review() {
    let rig = rig().await;
    rig.port.set_rules(reads(&["nodes"]));
    rig.visibility
        .refresh(&rig.cluster, &NamespaceSelection::All)
        .await;
    let node = Gvk::new("", "v1", "Node");
    let pv = Gvk::new("", "v1", "PersistentVolume");

    let ask = |gvk: &Gvk, plural: &str| {
        rig.visibility
            .is_listable_resource(&rig.cluster, gvk, Some(plural), None)
    };
    assert_eq!(ask(&node, "nodes"), Some(true));
    assert_eq!(ask(&pv, "persistentvolumes"), Some(false));
    assert_eq!(
        ask(&pod(), "pods"),
        None,
        "all namespaces: the cluster-wide review sees one probe namespace only"
    );
}
