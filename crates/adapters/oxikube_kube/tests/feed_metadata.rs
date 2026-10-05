//! Kind integration for E04-S03: a metadata-only feed over 2 000 pods carries names, labels
//! and owner references but no `spec`, stays live for changes, and one pod upgrades to the
//! full object on demand. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips
//! cleanly otherwise.
//!
//! The pods are unschedulable (`pending_pod`) and live in the test's own `oxi-test-<rand>`
//! namespace; they are owned by a ConfigMap of that namespace, so garbage collection keeps
//! them.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client};
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_kube::{FeedConfig, ReflectorFeed, StreamingLists};
use oxikube_ports::{Delta, DeltaBatch, WatchOptions};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::resources::{adapter, create_pods, pending_pod};

const PODS: usize = 2_000;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// 2 000 labelled pods owned by one ConfigMap; returns the owner's uid.
async fn create_owned_pods(client: &Client, namespace: &str, count: usize) -> String {
    let owners = Api::<ConfigMap>::namespaced(client.clone(), namespace);
    let owner = owners
        .create(
            &PostParams::default(),
            &serde_json::from_value(json!({"metadata": {"name": "owner"}})).unwrap(),
        )
        .await
        .expect("create the owner");
    let uid = owner.metadata.uid.expect("owner uid");
    let pods = (0..count)
        .map(|i| {
            let mut pod = pending_pod(&format!("meta-{i}"), &[("app", "meta"), ("tier", "web")]);
            pod.metadata.owner_references = Some(vec![
                serde_json::from_value(json!({
                    "apiVersion": "v1", "kind": "ConfigMap", "name": "owner",
                    "uid": uid, "controller": true,
                }))
                .unwrap(),
            ]);
            pod
        })
        .collect();
    create_pods(client, namespace, pods).await;
    uid
}

/// The opening batch of `feed`: the complete list.
async fn first_list(feed: &mut ReflectorFeed) -> Vec<Resource> {
    let batch = tokio::time::timeout(Duration::from_secs(60), feed.next())
        .await
        .expect("the opening batch in time")
        .expect("the feed is open")
        .expect("the opening batch");
    match batch.deltas.into_iter().next() {
        Some(Delta::Restarted(all)) => all,
        other => panic!("the first delta is the list, got {other:?}"),
    }
}

async fn next_batch(feed: &mut ReflectorFeed) -> DeltaBatch<Resource> {
    tokio::time::timeout(Duration::from_secs(30), feed.next())
        .await
        .expect("a batch in time")
        .expect("the feed is open")
        .expect("a batch, not an error")
}

fn json_bytes(resources: &[Resource]) -> usize {
    resources.iter().map(|r| r.json.to_string().len()).sum()
}

#[tokio::test]
async fn two_thousand_pods_arrive_as_metadata_and_one_upgrades_to_full() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let owner_uid = create_owned_pods(&client, ns.name(), PODS).await;

    let resources = adapter(&client);
    let scope = WatchScope::Namespaces(vec![ns.name().to_owned()]);
    let mut feed = resources
        .metadata_feed(&pod_gvk(), &scope, &WatchOptions::default())
        .await
        .expect("open the metadata feed");
    let listed = first_list(&mut feed).await;

    // Names, labels and owner references; no spec, no status; marked partial.
    assert_eq!(listed.len(), PODS);
    for pod in &listed {
        assert!(pod.is_partial(), "{pod:?}");
        assert_eq!(pod.kind, pod_gvk());
        assert!(pod.name().starts_with("meta-"));
        assert_eq!(pod.namespace(), Some(ns.name()));
        assert_eq!(pod.meta.labels.get("app").map(|v| &**v), Some("meta"));
        let owner = pod.meta.controller_ref().expect("owner reference");
        assert_eq!((&*owner.kind, &*owner.name), ("ConfigMap", "owner"));
        assert_eq!(&*owner.uid, owner_uid);
        assert!(pod.get("/spec").is_none(), "{}", pod.name());
        assert!(pod.get("/status").is_none(), "{}", pod.name());
    }

    // The cheap list is clearly smaller than the same list in full.
    let full_feed = resources
        .reflector_feed(&pod_gvk(), &scope, &WatchOptions::default())
        .await
        .expect("open the full feed");
    let mut full_feed = full_feed;
    let full = first_list(&mut full_feed).await;
    assert_eq!(full.len(), PODS);
    assert!(
        full.iter()
            .all(|p| !p.is_partial() && p.get("/spec").is_some())
    );
    let (meta_bytes, full_bytes) = (json_bytes(&listed), json_bytes(&full));
    eprintln!(
        "metadata feed: {PODS} pods, {meta_bytes} JSON bytes vs {full_bytes} full ({:.0}%)",
        100.0 * meta_bytes as f64 / full_bytes as f64
    );
    assert!(meta_bytes * 2 < full_bytes, "{meta_bytes} vs {full_bytes}");
    drop(full_feed);

    // Upgrade one pod: the full object, with its spec; the feed keeps running.
    let partial = listed.iter().find(|p| p.name() == "meta-17").unwrap();
    let upgraded = resources.upgrade(partial).await.expect("upgrade");
    assert!(!upgraded.is_partial());
    assert_eq!(upgraded.meta.uid, partial.meta.uid);
    assert_eq!(
        upgraded.get_str("/spec/containers/0/name"),
        Some("pause"),
        "the upgraded pod has its spec"
    );

    // A change still arrives as a partial object.
    let api = Api::<Pod>::namespaced(client.as_ref().clone(), ns.name());
    let patch = json!({"metadata": {"labels": {"touched": "yes"}}});
    api.patch("meta-17", &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .expect("relabel");
    loop {
        let batch = next_batch(&mut feed).await;
        let changed = batch.into_iter().find_map(|d| match d {
            Delta::Applied(r) if r.name() == "meta-17" && r.meta.labels.contains_key("touched") => {
                Some(r)
            }
            _ => None,
        });
        if let Some(changed) = changed {
            assert!(changed.is_partial());
            assert!(changed.get("/spec").is_none());
            break;
        }
    }
}

#[tokio::test]
async fn paged_lists_and_selectors_work_for_metadata_feeds_on_a_real_server() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_owned_pods(&client, ns.name(), 40).await;
    create_pods(
        &client,
        ns.name(),
        vec![pending_pod("other", &[("app", "x")])],
    )
    .await;

    let resources = adapter(&client).with_feed_config(FeedConfig {
        streaming_lists: StreamingLists::Never,
        page_size: 7,
        ..FeedConfig::default()
    });
    let scope = WatchScope::Namespaces(vec![ns.name().to_owned()]);
    let mut feed = resources
        .metadata_feed(
            &pod_gvk(),
            &scope,
            &WatchOptions::default().labels("app=meta"),
        )
        .await
        .expect("open the metadata feed");
    let listed = first_list(&mut feed).await;
    assert_eq!(listed.len(), 40, "paged list, selected pods only");
    assert!(
        listed
            .iter()
            .all(|p| p.is_partial() && p.get("/spec").is_none())
    );
}
