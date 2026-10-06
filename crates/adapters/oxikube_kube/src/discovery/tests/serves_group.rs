use oxikube_ports::DiscoveryPort;

use super::fake::{Behaviour, FakeApiServer};

const METRICS: &str = "metrics.k8s.io";

#[tokio::test]
async fn serves_group_discovers_first_instead_of_reading_an_empty_registry() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    server.add_group(METRICS, "v1beta1", "PodMetrics", "pods");
    let discovery = server.discovery();
    assert!(discovery.registry().is_empty());

    assert!(discovery.serves_group(METRICS).await);
    assert!(!discovery.serves_group("absent.example.dev").await);
}

#[tokio::test]
async fn serves_group_beside_a_running_discover_does_not_miss_the_group() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    server.add_group(METRICS, "v1beta1", "PodMetrics", "pods");
    let discovery = server.discovery();

    // The session manager's shape: discovery and the capability probe side by side, the probe
    // asking before the registry is filled.
    let (kinds, served) = futures::join!(discovery.discover(), discovery.serves_group(METRICS));
    kinds.expect("discover");
    assert!(
        served,
        "the flag must not depend on which future finishes first"
    );
}

#[tokio::test]
async fn serves_group_does_not_rediscover_once_discovered() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let before = server.request_count();

    assert!(!discovery.serves_group(METRICS).await);
    assert_eq!(server.request_count(), before);
}

#[tokio::test]
async fn serves_group_is_false_when_discovery_fails() {
    let server = FakeApiServer::new(Behaviour::AggregatedFails(500));
    server.fail_path("/api", 500);
    server.fail_path("/apis", 500);
    let discovery = server.discovery();
    assert!(!discovery.serves_group(METRICS).await);
}
