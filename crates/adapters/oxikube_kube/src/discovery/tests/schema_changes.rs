//! A CRD edit that changes no kind record (its schema) still reaches subscribers: the registry
//! diff is empty then, so `SchemasChanged` is the only signal that cached schemas are stale.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_ports::{DiscoveryEvent, DiscoveryPort};
use tokio::sync::broadcast::error::TryRecvError;

use super::fake::{Behaviour, FakeApiServer};

#[tokio::test]
async fn a_crd_refresh_that_changes_no_kind_still_signals_a_schema_change() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let mut diffs = discovery.registry_changes();
    let mut schemas = discovery.schema_changes();

    assert!(discovery.refresh_after_crd_change().await);

    assert!(
        matches!(diffs.try_recv(), Err(TryRecvError::Empty)),
        "the served kinds are the same: no registry diff"
    );
    assert_eq!(schemas.try_recv(), Ok(()));
}

#[tokio::test]
async fn a_failed_crd_refresh_signals_nothing_until_it_succeeds() {
    let server = FakeApiServer::new(Behaviour::AggregatedFails(500));
    server.fail_path("/api", 500);
    server.fail_path("/apis", 500);
    let discovery = server.discovery();
    let mut schemas = discovery.schema_changes();

    assert!(!discovery.refresh_after_crd_change().await);
    assert!(matches!(schemas.try_recv(), Err(TryRecvError::Empty)));
}

#[tokio::test]
async fn subscribers_hear_a_schema_change_as_an_event() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let mut events = discovery.subscribe();

    assert!(discovery.refresh_after_crd_change().await);

    let heard = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if event == DiscoveryEvent::SchemasChanged {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(heard, Ok(true));
}
