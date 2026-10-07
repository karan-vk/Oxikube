use std::time::Duration;

use futures::future::join_all;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_ports::DiscoveryPort;

use super::fake::{Behaviour, FakeApiServer};
use crate::discovery::{DiscoveryConfig, KubeDiscovery};

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

#[tokio::test]
async fn hit_is_served_from_the_cache() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let requests = server.request_count();

    let found = discovery
        .resolve(&pod())
        .await
        .expect("resolve")
        .expect("pod");
    assert_eq!(found.plural, "pods");
    assert_eq!(
        server.request_count(),
        requests,
        "a hit must not touch the server"
    );
}

#[tokio::test]
async fn miss_rediscovers_once_and_finds_a_kind_added_since() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let before = server.request_count();

    server.add_group("later.example.dev", "v1", "Gadget", "gadgets");
    let gadget = Gvk::new("later.example.dev", "v1", "Gadget");
    let found = discovery.resolve(&gadget).await.expect("resolve");

    assert_eq!(found.map(|k| k.plural), Some("gadgets".to_owned()));
    assert_eq!(server.request_count() - before, 2, "one aggregated refresh");
}

#[tokio::test]
async fn unknown_kind_is_none_after_one_refresh_then_cools_down() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let before = server.request_count();
    let unknown = Gvk::new("nope.example.dev", "v1", "Nope");

    assert!(
        discovery
            .resolve(&unknown)
            .await
            .expect("resolve")
            .is_none()
    );
    assert_eq!(server.request_count() - before, 2);

    assert!(
        discovery
            .resolve(&unknown)
            .await
            .expect("resolve")
            .is_none()
    );
    assert_eq!(
        server.request_count() - before,
        2,
        "inside the cooldown a miss does not refresh"
    );
}

#[tokio::test(start_paused = true)]
async fn a_miss_after_the_cooldown_refreshes_again() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    discovery.discover().await.expect("discover");
    let unknown = Gvk::new("nope.example.dev", "v1", "Nope");
    assert!(
        discovery
            .resolve(&unknown)
            .await
            .expect("resolve")
            .is_none()
    );
    let after_first = server.request_count();

    tokio::time::advance(DiscoveryConfig::default().miss_cooldown + Duration::from_millis(1)).await;
    assert!(
        discovery
            .resolve(&unknown)
            .await
            .expect("resolve")
            .is_none()
    );
    assert_eq!(server.request_count() - after_first, 2);
}

#[tokio::test]
async fn concurrent_misses_share_one_refresh() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();

    let gvk = pod();
    let lookups = (0..8).map(|_| discovery.resolve(&gvk));
    for found in join_all(lookups).await {
        assert!(found.expect("resolve").is_some());
    }
    assert_eq!(
        server.request_count(),
        2,
        "an empty registry plus eight misses is one refresh"
    );
}

#[tokio::test]
async fn refresh_failure_during_a_miss_is_an_error_not_none() {
    let server = FakeApiServer::new(Behaviour::AggregatedFails(401));
    let err = server
        .discovery()
        .resolve(&pod())
        .await
        .expect_err("must fail");
    assert_eq!(err.kind(), ErrorKind::Auth);
}

#[tokio::test]
async fn empty_version_resolves_to_the_preferred_version() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery = server.discovery();
    let found = discovery
        .resolve(&Gvk::new("autoscaling", "", "HorizontalPodAutoscaler"))
        .await
        .expect("resolve")
        .expect("hpa");
    assert_eq!(&*found.gvk.version, "v2");
}

#[tokio::test]
async fn api_resource_is_available_for_dynamic_apis() {
    let discovery = FakeApiServer::new(Behaviour::Aggregated).discovery();
    let resource = discovery
        .resolve_api_resource(&Gvk::new("test.oxikube.dev", "v1", "Widget"))
        .await
        .expect("resolve")
        .expect("widget");
    assert_eq!(resource.api_version, "test.oxikube.dev/v1");
    assert_eq!(resource.plural, "widgets");
    assert_eq!(resource.kind, "Widget");
}

#[tokio::test]
async fn subscribers_see_the_diff_of_each_changing_refresh() {
    let server = FakeApiServer::new(Behaviour::Aggregated);
    let discovery: KubeDiscovery = server.discovery();
    let mut changes = discovery.registry_changes();

    discovery.refresh().await.expect("first refresh");
    let first = changes.try_recv().expect("first diff");
    assert!(first.added.len() > 10 && first.removed.is_empty());

    discovery.refresh().await.expect("unchanged refresh");
    assert!(
        changes.try_recv().is_err(),
        "an unchanged registry publishes nothing"
    );

    server.add_group("later.example.dev", "v1", "Gadget", "gadgets");
    discovery.refresh().await.expect("refresh with a new group");
    let diff = changes.try_recv().expect("diff");
    let added: Vec<_> = diff.added.iter().map(|k| k.gvk.to_string()).collect();
    assert_eq!(added, ["later.example.dev/v1/Gadget"]);
    assert_eq!(
        discovery
            .registry()
            .get(&Gvk::new("later.example.dev", "v1", "Gadget"))
            .map(|k| k.plural.as_str()),
        Some("gadgets")
    );
}
