//! A Service's detail queries are applied by the API server (E12-S13): the EndpointSlices by
//! `kubernetes.io/service-name` and the pods by the Service's selector, through the real
//! `KubeResources` reflector feed and `ServiceBackends`.
//!
//! Two Services with different selectors, and pods behind neither scheduler (a name no scheduler
//! answers to, so nothing starts), all in the test's own `oxi-test-<rand>` namespace. The store's
//! `total` (what its cache holds before any in-app filter) is the proof: with the selectors it is
//! the matches, not everything the namespace has.

use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::Api;
use kube::api::PostParams;
use oxikube_app::store::{
    ResourceStore, Spawner, StoreObject, StoreOptions, StorePorts, StoreRuntime, Subscription,
};
use oxikube_app::{PodsPlan, ServiceBackends};
use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::view::ServiceSummary;
use oxikube_kube::{KubeDiscovery, KubeResources};
use oxikube_testkit::FakeClockPort;
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use crate::cluster::Kind;

const APP: &str = "oxikube.test/app";

fn service(name: &str, app: &str) -> Service {
    serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": {"name": name},
        "spec": {
            "selector": {APP: app},
            "ports": [{"name": "http", "port": 80, "targetPort": 8080}],
        },
    }))
    .expect("a service")
}

fn pod(name: &str, app: &str) -> Pod {
    serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "labels": {APP: app}},
        "spec": {
            "schedulerName": "oxikube-test-no-scheduler",
            "terminationGracePeriodSeconds": 1,
            "containers": [{"name": "pause", "image": "registry.k8s.io/pause:3.10"}],
        },
    }))
    .expect("a pod")
}

/// Applies items until `ready` holds for the rows' names and the store's total.
async fn until(sub: &mut Subscription, what: &str, ready: impl Fn(&[String], usize) -> bool) {
    let mut rows = Vec::new();
    let deadline = tokio::time::Instant::now() + crate::DEADLINE;
    loop {
        let item = tokio::time::timeout_at(deadline, sub.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}: have {:?}", names(&rows)))
            .expect("the stream does not end");
        item.apply_to(&mut rows);
        if ready(&names(&rows), item.total) {
            return;
        }
    }
}

fn names(rows: &[Arc<StoreObject>]) -> Vec<String> {
    rows.iter().map(|o| o.name().to_owned()).collect()
}

#[tokio::test]
async fn the_api_server_applies_the_detail_queries_of_a_service() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let services = Api::<Service>::namespaced(client.clone(), ns.name());
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    let post = PostParams::default();
    for (name, app) in [("web-a", "a"), ("web-b", "b")] {
        services
            .create(&post, &service(name, app))
            .await
            .expect("create a Service");
    }
    for (name, app) in [("pod-a1", "a"), ("pod-a2", "a"), ("pod-b1", "b")] {
        pods.create(&post, &pod(name, app))
            .await
            .expect("create a Pod");
    }

    // The Service as the store would hand it to the detail.
    let created = services.get("web-a").await.expect("get the Service");
    let summary = ServiceSummary::from_resource(
        &Resource::from_json(serde_json::to_value(&created).expect("json")).expect("a resource"),
    )
    .expect("a Service");
    let backends = ServiceBackends::of(&summary);
    let PodsPlan::Query(pod_query) = backends.pods.clone() else {
        panic!("web-a selects pods");
    };

    let reader = Arc::new(KubeResources::new(
        client.clone(),
        KubeDiscovery::new(client),
    ));
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let store = ResourceStore::new(
        ClusterId::new("kind-smoke", &kind.context),
        StorePorts {
            resources: reader.clone(),
            tables: reader,
        },
        StoreRuntime {
            spawner,
            clock: Arc::new(FakeClockPort::default()),
            probe: None,
        },
        StoreOptions::default(),
    );

    let mut pod_sub = store.subscribe(pod_query);
    until(&mut pod_sub, "the pods of web-a", |names, total| {
        names == ["pod-a1", "pod-a2"] && total == 2
    })
    .await;

    // The EndpointSlice controller labels a Service's slices (even the empty placeholder one)
    // with `kubernetes.io/service-name`; web-b's slice is not web-a's.
    let mut slice_sub = store.subscribe(backends.slices.clone());
    until(&mut slice_sub, "web-a's EndpointSlice", |names, total| {
        total == 1 && names.iter().all(|n| n.starts_with("web-a-"))
    })
    .await;

    let feeds = store.feeds();
    let selectors: Vec<String> = feeds
        .iter()
        .filter_map(|f| f.key.selector.as_ref().map(ToString::to_string))
        .collect();
    assert!(
        selectors.contains(&format!("{APP}=a")),
        "a pod feed keyed with the Service's selector: {selectors:?}"
    );
    assert!(
        selectors.contains(&"kubernetes.io/service-name=web-a".to_owned()),
        "a slice feed keyed with the service name: {selectors:?}"
    );
    assert!(
        feeds.iter().all(|f| f.key.selector.is_some()),
        "no unfiltered feed was started: {feeds:?}"
    );
}
