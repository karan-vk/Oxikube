use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_ports::DiscoveryPort;

use super::fake::{Behaviour, FakeApiServer};
use crate::discovery::{DiscoveryConfig, KubeDiscovery};

async fn discover(server: &FakeApiServer) -> Vec<ResourceKind> {
    server.discovery().discover().await.expect("discover")
}

#[tokio::test]
async fn aggregated_discovery_is_two_requests() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let kinds = discover(&server).await;

    let mut requests = server.requests();
    requests.sort();
    assert_eq!(requests, ["/api (aggregated)", "/apis (aggregated)"]);
    assert!(kinds.iter().any(|k| &*k.gvk.kind == "Deployment"));
}

#[tokio::test]
async fn discover_lists_every_version_with_the_preferred_flagged() {
    let kinds = discover(&FakeApiServer::new(Behaviour::Aggregated)).await;
    let hpa: Vec<_> = kinds
        .iter()
        .filter(|k| &*k.gvk.kind == "HorizontalPodAutoscaler")
        .map(|k| (k.gvk.version.to_string(), k.preferred))
        .collect();
    assert_eq!(hpa, [("v1".to_owned(), false), ("v2".to_owned(), true)]);
    let widget = kinds
        .iter()
        .find(|k| &*k.gvk.kind == "Widget")
        .expect("widget");
    assert!(widget.supports(Verb::List) && widget.is_watchable());
}

#[tokio::test]
async fn legacy_server_yields_the_same_registry() {
    let aggregated = discover(&FakeApiServer::new(Behaviour::Aggregated)).await;

    let old = FakeApiServer::new(Behaviour::IgnoreAccept);
    let legacy = discover(&old).await;

    assert_eq!(legacy, aggregated);
    // Two aggregated attempts that came back in the legacy shape, then N + 2 legacy requests.
    assert!(old.requests().contains(&"/apis/apps/v1".to_owned()));
}

#[tokio::test]
async fn aggregated_404_falls_back_to_legacy() {
    let aggregated = discover(&FakeApiServer::new(Behaviour::Aggregated)).await;

    let server = FakeApiServer::new(Behaviour::AggregatedFails(404));
    assert_eq!(discover(&server).await, aggregated);
    assert!(server.requests().contains(&"/api/v1".to_owned()));
}

#[tokio::test]
async fn aggregated_406_is_unsupported_and_falls_back() {
    let server = FakeApiServer::new(Behaviour::AggregatedFails(406));
    assert!(!discover(&server).await.is_empty());
}

#[tokio::test]
async fn credential_failures_do_not_fall_back() {
    for (status, kind) in [(401, ErrorKind::Auth), (403, ErrorKind::Forbidden)] {
        let server = FakeApiServer::new(Behaviour::AggregatedFails(status));
        let err = server.discovery().discover().await.expect_err("must fail");
        assert_eq!(err.kind(), kind);
        assert!(
            server
                .requests()
                .iter()
                .all(|r| r.ends_with("(aggregated)")),
            "{:?}",
            server.requests()
        );
    }
}

#[tokio::test]
async fn unavailable_server_is_a_retryable_network_error() {
    let server = FakeApiServer::new(Behaviour::AggregatedFails(503));
    let err = server.discovery().discover().await.expect_err("must fail");
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.is_retryable());
}

#[tokio::test]
async fn legacy_skips_a_group_version_that_fails() {
    let server = FakeApiServer::new(Behaviour::IgnoreAccept);
    server.fail_path("/apis/autoscaling/v1", 503);
    let kinds = discover(&server).await;

    assert!(kinds.iter().any(|k| &*k.gvk.kind == "Deployment"));
    assert!(
        kinds
            .iter()
            .any(|k| k.gvk == Gvk::new("autoscaling", "v2", "HorizontalPodAutoscaler"))
    );
    assert!(
        kinds
            .iter()
            .all(|k| k.gvk != Gvk::new("autoscaling", "v1", "HorizontalPodAutoscaler"))
    );
}

#[tokio::test]
async fn legacy_with_every_group_version_failing_is_an_error() {
    let server = FakeApiServer::new(Behaviour::IgnoreAccept);
    for path in [
        "/api/v1",
        "/apis/apps/v1",
        "/apis/autoscaling/v2",
        "/apis/autoscaling/v1",
        "/apis/authentication.k8s.io/v1",
        "/apis/test.oxikube.dev/v1",
    ] {
        server.fail_path(path, 403);
    }
    let err = server.discovery().discover().await.expect_err("must fail");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn server_version_is_read_from_version_endpoint() {
    let version = FakeApiServer::new(Behaviour::Aggregated)
        .discovery()
        .server_version()
        .await
        .expect("version");
    assert_eq!(version.git_version, "v1.34.1");
    assert!(version.at_least(1, 30));
}

#[tokio::test]
async fn aggregated_can_be_turned_off() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let config = DiscoveryConfig {
        aggregated: false,
        ..DiscoveryConfig::default()
    };
    let legacy = KubeDiscovery::with_config(server.client(), config)
        .discover()
        .await
        .expect("discover");

    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.ends_with("(aggregated)")),
        "{:?}",
        server.requests()
    );
    assert_eq!(
        legacy,
        discover(&FakeApiServer::new(Behaviour::Aggregated)).await
    );
}
