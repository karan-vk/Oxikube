//! The real source ([`KubeResources`]) behind the registry, over the in-process fake API:
//! bytes are the feed's own response bodies, a degrade asks the server for metadata, a Table
//! request opens the Table feed.

use oxikube_ports::FeedVariant;
use serde_json::Value;

use super::*;
use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::FakeApi;
use crate::feed::{FeedConfig, StreamingLists};
use crate::resources::KubeResources;

const PODS: &str = "/api/v1/namespaces/default/pods";

/// A server with legacy discovery for `Pod`.
fn server() -> FakeApi {
    let api = FakeApi::new();
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply("/apis", 200, json!({"kind": "APIGroupList", "groups": []}));
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [
            {"name": "pods", "singularName": "", "namespaced": true, "kind": "Pod",
             "verbs": ["get", "list", "watch"]},
        ]}),
    );
    api
}

fn resources(api: &FakeApi) -> KubeResources {
    let discovery = KubeDiscovery::with_config(
        api.client(),
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );
    KubeResources::new(api.client(), discovery).with_feed_config(FeedConfig {
        streaming_lists: StreamingLists::Never,
        ..FeedConfig::default()
    })
}

fn pod_item(name: &str, rv: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "default", "uid": format!("u-{name}"),
                     "resourceVersion": rv},
        "spec": {"containers": [{"name": "c", "image": "busybox"}]},
    })
}

fn pod_list(items: Vec<Value>) -> Value {
    json!({"kind": "PodList", "apiVersion": "v1", "metadata": {"resourceVersion": "10"},
           "items": items})
}

fn kube_registry(api: &FakeApi, config: BudgetConfig) -> FeedRegistry {
    FeedRegistry::for_resources(cluster(), resources(api), config)
}

#[tokio::test]
async fn bytes_are_the_response_bodies_of_the_feed() {
    let api = server();
    let list = pod_list(vec![pod_item("a", "1"), pod_item("b", "2")]);
    let event = json!({"type": "ADDED", "object": pod_item("c", "11")});
    api.reply(PODS, 200, list.clone());
    api.reply_watch(PODS, 200, std::slice::from_ref(&event));
    let registry = kube_registry(&api, roomy());

    let mut lease = registry.subscribe(full(pods(), "default")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let first = next(&mut stream).await.unwrap().unwrap();
    assert!(first.contains_restart());
    let second = next(&mut stream).await.unwrap().unwrap();
    assert_eq!(second.len(), 1);

    let stats = registry.stats();
    let expected = list.to_string().len() + event.to_string().len();
    assert_eq!(
        stats.bytes, expected as u64,
        "the list and the watch body, nothing else"
    );
    assert_eq!((stats.objects, stats.events, stats.restarts), (3, 1, 1));
}

#[tokio::test]
async fn a_degraded_feed_asks_the_server_for_metadata_only() {
    let api = server();
    let meta = |name: &str| {
        json!({"apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
               "metadata": {"name": name, "namespace": "default", "uid": format!("u-{name}"),
                            "resourceVersion": "1"}})
    };
    api.reply(
        PODS,
        200,
        json!({"kind": "PartialObjectMetadataList", "apiVersion": "meta.k8s.io/v1",
               "metadata": {"resourceVersion": "10"}, "items": [meta("a")]}),
    );
    let config = BudgetConfig {
        metadata_above: 0,
        ..roomy()
    };
    let registry = kube_registry(&api, config);
    let mut lease = registry.subscribe(full(pods(), "default")).await.unwrap();
    assert!(lease.is_degraded());
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let first = next(&mut stream).await.unwrap().unwrap();
    let Some(Delta::Restarted(all)) = first.deltas.first() else {
        panic!("the opening list")
    };
    assert!(all.iter().all(Resource::is_partial));
    let list = api
        .requests()
        .into_iter()
        .find(|r| r.path == PODS && !r.is_watch())
        .expect("the list request");
    assert!(
        list.accept
            .as_deref()
            .is_some_and(|a| a.contains("as=PartialObjectMetadata")),
        "{:?}",
        list.accept
    );
}

#[tokio::test]
async fn a_table_request_opens_the_table_feed() {
    let api = server();
    // A plain list: the server ignored the Table `Accept`, so the feed falls back to objects.
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_item("a", "1"), pod_item("b", "2")]),
    );
    let registry = kube_registry(&api, roomy());
    let request = FeedRequest::new(pods(), FeedVariant::Table).in_namespace(Some("default"));
    let mut lease = registry.subscribe(request).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_table().unwrap();
    let first = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(first.rows.contains_restart());
    let list = api
        .requests()
        .into_iter()
        .find(|r| r.path == PODS)
        .expect("the table list");
    assert!(
        list.accept
            .as_deref()
            .is_some_and(|a| a.contains("as=Table")),
        "{:?}",
        list.accept
    );
    assert!(
        list.query.contains("includeObject=Metadata"),
        "{}",
        list.query
    );
    let stats = registry.stats();
    assert_eq!((stats.objects, stats.restarts), (2, 1));
    assert!(stats.bytes > 0);
}

#[tokio::test]
async fn an_unserved_kind_is_the_sources_error() {
    let api = server();
    let registry = kube_registry(&api, roomy());
    let request = full(Gvk::new("example.com", "v1", "Widget"), "default");
    let err = registry.subscribe(request).await.unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Unsupported);
    assert_eq!(registry.stats().feeds, 0);
}
