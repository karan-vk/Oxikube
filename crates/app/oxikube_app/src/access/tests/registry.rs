//! The registry hands one service per connection.

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{FakeClusterConnectorPort, FakeClusterSourcePort};

use crate::access::AccessRegistry;
use crate::session::ClusterSessionManager;

use super::*;

fn context() -> ClusterContext {
    let name = ContextName::new("a");
    let cluster = ClusterId::new("/home/me/.kube/config", &name);
    ClusterContext::new(cluster, name, SourceId("kubeconfig".into()))
}

#[tokio::test]
async fn a_service_is_shared_per_connection_and_forgotten_on_close() {
    let ctx = context();
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([ctx.clone()]));
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(connector, source, clock.clone());
    let registry = AccessRegistry::new(clock);
    manager.open(&ctx, Default::default());
    assert!(
        registry
            .for_session(&ctx.cluster, &manager.get(&ctx.cluster).expect("session"))
            .is_none(),
        "not connected: no port"
    );

    manager.connect(&ctx.cluster).await.expect("connect");
    let session = manager.get(&ctx.cluster).expect("session");
    let first = registry
        .for_session(&ctx.cluster, &session)
        .expect("service");
    first.rules(Some("dev")).await.expect("rules");
    let same = registry
        .for_session(&ctx.cluster, &session)
        .expect("service");
    assert!(
        same.peek_rules(Some("dev")).is_some(),
        "same connection, same cache"
    );

    // A closed cluster is forgotten; asking again builds a new service. (A reconnect that
    // yields a different port does the same: the registry compares the port by identity.)
    registry.forget(&ctx.cluster);
    let fresh = registry
        .for_session(&ctx.cluster, &session)
        .expect("service");
    assert!(
        fresh.peek_rules(Some("dev")).is_none(),
        "forgotten: empty cache"
    );
}

/// Waits (on the test runtime) until `done`, yielding so the follower runs.
async fn until(done: impl Fn() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert!(done(), "the follower did not catch up");
}

/// A closed or disconnected cluster's service, and the port (kube client pool, discovery) it
/// pins, is dropped by the follower; nothing has to call `forget` by hand.
#[tokio::test]
async fn the_follower_releases_closed_and_disconnected_clusters() {
    let first = context();
    let other_name = ContextName::new("b");
    let second = ClusterContext::new(
        ClusterId::new("/home/me/.kube/config", &other_name),
        other_name,
        SourceId("kubeconfig".into()),
    );
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source =
        Arc::new(FakeClusterSourcePort::new().with_contexts([first.clone(), second.clone()]));
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(connector, source, clock.clone());
    let registry = AccessRegistry::new(clock);
    let follower = tokio::spawn(registry.follow(&manager));
    for ctx in [&first, &second] {
        manager.open(ctx, Default::default());
        manager.connect(&ctx.cluster).await.expect("connect");
        let session = manager.get(&ctx.cluster).expect("session");
        registry
            .for_session(&ctx.cluster, &session)
            .expect("service");
    }
    assert_eq!(registry.len(), 2);

    assert!(manager.close(&first.cluster));
    until(|| registry.len() == 1).await;

    manager.disconnect(&second.cluster).expect("disconnect");
    until(|| registry.is_empty()).await;

    drop(registry);
    manager.open(&first, Default::default());
    assert!(manager.close(&first.cluster));
    tokio::time::timeout(std::time::Duration::from_secs(5), follower)
        .await
        .expect("the follower ends with the registry")
        .expect("no panic");
}
