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
