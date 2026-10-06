//! The filter bar's label selector is applied by the API server (E07-S04): the store's feed is
//! re-keyed with `-l app=a`, so the cache holds only what the server returned for it.
//!
//! The objects are ConfigMaps in the test's own `oxi-test-<rand>` namespace (no scheduler, no
//! pods), the store reads them through the real `KubeResources` reflector feed, and the store's
//! `total` (what its cache holds before the in-app filter) is the proof: with the selector it is
//! the matches, not every ConfigMap of the namespace. Text filters stay client-side and never
//! re-key.

use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::Api;
use kube::api::{ObjectMeta, PostParams};
use oxikube_app::store::filter::parse;
use oxikube_app::store::{
    ResourceStore, RowChange, Spawner, StoreOptions, StorePorts, StoreQuery, StoreRuntime,
    Subscription,
};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::session::WatchScope;
use oxikube_kube::{KubeDiscovery, KubeResources};
use oxikube_testkit::FakeClockPort;
use oxikube_testkit::integration::TestNamespace;

use crate::cluster::Kind;

fn gvk() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

async fn create(api: &Api<ConfigMap>, name: &str, app: &str) {
    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            labels: Some([("oxikube.test/app".to_owned(), app.to_owned())].into()),
            ..ObjectMeta::default()
        },
        ..ConfigMap::default()
    };
    api.create(&PostParams::default(), &cm)
        .await
        .expect("create a ConfigMap");
}

/// Applies items until the subscription shows `want` rows (by name) and `total` objects held.
async fn until(sub: &mut Subscription, want: &[&str], total: usize) {
    let mut names: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + crate::DEADLINE;
    loop {
        let item = tokio::time::timeout_at(deadline, sub.next())
            .await
            .unwrap_or_else(|_| panic!("timed out: have {names:?}, want {want:?} of {total}"))
            .expect("the stream does not end");
        // Every change here is a filter or scope change, which is a snapshot; a state-only item
        // keeps the names.
        match &item.rows {
            RowChange::Snapshot(rows) => {
                names = rows.iter().map(|o| o.name().to_owned()).collect();
            }
            RowChange::Unchanged => {}
            RowChange::Ops(ops) => panic!("unexpected ops {ops:?}"),
        }
        if names == want && item.total == total {
            return;
        }
    }
}

#[tokio::test]
async fn a_label_selector_is_applied_by_the_api_server() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let api = Api::<ConfigMap>::namespaced(client.clone(), ns.name());
    for (name, app) in [("cm-a1", "a"), ("cm-a2", "a"), ("cm-b1", "b")] {
        create(&api, name, app).await;
    }

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
    let query = StoreQuery::new(gvk(), WatchScope::Namespaces(vec![ns.name().to_owned()]));
    let mut sub = store.subscribe(query);
    // The namespace also holds the ConfigMap Kubernetes adds to every namespace
    // (`kube-root-ca.crt`), which the unfiltered feed shows too.
    let mine = |input: &str| parse(input).expect("parses").parts();

    // A text filter is the store's: the feed holds all four objects.
    let parts = mine("cm-a");
    sub.set_filter_parts(parts.clone(), parts.sort(None));
    until(&mut sub, &["cm-a1", "cm-a2"], 4).await;

    // A label selector re-keys the feed: the server sends only app=a, so the store holds two.
    let parts = mine("-l oxikube.test/app=a");
    sub.set_filter_parts(parts.clone(), parts.sort(None));
    until(&mut sub, &["cm-a1", "cm-a2"], 2).await;
    let feeds = store.feeds();
    assert!(
        feeds.iter().any(
            |f| f.key.selector.as_ref().map(ToString::to_string).as_deref()
                == Some("oxikube.test/app=a")
        ),
        "a feed keyed with the selector: {feeds:?}"
    );

    // Another selector, another feed; and no selector again reads everything.
    let parts = mine("-l oxikube.test/app=b");
    sub.set_filter_parts(parts.clone(), parts.sort(None));
    until(&mut sub, &["cm-b1"], 1).await;
    let parts = mine("");
    sub.set_filter_parts(parts.clone(), parts.sort(None));
    until(
        &mut sub,
        &["cm-a1", "cm-a2", "cm-b1", "kube-root-ca.crt"],
        4,
    )
    .await;
}
