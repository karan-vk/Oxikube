//! Kind integration for E04-S14: the `ResourcePort` methods the per-story suites reach only
//! below the trait, driven here through `Arc<dyn ResourcePort>` as the app holds the adapter:
//! `list_metadata` (pages, selectors, cluster scope, a custom resource, `Forbidden`), `watch`
//! (full and metadata-only feeds folding create, update and delete) and `create_subresource`
//! (an eviction posted by name, dry run, and the error kinds). Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
//!
//! Pods are unschedulable (`pending_pod`): they cost the API server, not the shared node, and
//! live in the test's own `oxi-test-<rand>` namespace. The `Widget` fixtures are only read.
#![cfg(feature = "integration")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures::StreamExt as _;
use k8s_openapi::api::rbac::v1::PolicyRule;
use oxikube_domain::ids::{ContextName, Gvk};
use oxikube_domain::{ErrorKind, Resource};
use oxikube_ports::{
    DeleteOptions, Delta, ListOptions, Patch, ResourcePort, Subresource, WatchFeed, WatchOptions,
    WriteOptions,
};
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use common::resources::{adapter, create_pods, pending_pod};
use common::subresources::pod_gvk;
use common::{DEADLINE, TestServiceAccount, wait_in};

fn port(client: &kube::Client) -> Arc<dyn ResourcePort> {
    Arc::new(adapter(client))
}

/// Every name of a `list_metadata` walk, page by page; fails on a duplicate.
async fn metadata_names(
    port: &dyn ResourcePort,
    kind: &Gvk,
    namespace: Option<&str>,
    mut options: ListOptions,
) -> (Vec<String>, usize) {
    let (mut names, mut pages) = (Vec::new(), 0);
    loop {
        let page = port
            .list_metadata(kind, namespace, &options)
            .await
            .expect("list_metadata");
        pages += 1;
        names.extend(page.items.iter().map(|meta| meta.name.to_string()));
        match page.continue_token.filter(|t| !t.is_empty()) {
            Some(token) => options = options.continue_from(token),
            None => break,
        }
    }
    let unique: BTreeSet<_> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "duplicates across pages: {names:?}"
    );
    names.sort();
    (names, pages)
}

#[tokio::test]
async fn list_metadata_pages_selects_and_reads_custom_and_cluster_kinds() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port = port(&client);
    let mut pods: Vec<_> = (1..=5)
        .map(|i| pending_pod(&format!("a-{i}"), &[("tier", "a")]))
        .collect();
    pods.push(pending_pod("b-1", &[("tier", "b")]));
    create_pods(&client, ns.name(), pods).await;

    // A label selector, two per page: three pages, five names, no duplicates.
    let (names, pages) = metadata_names(
        port.as_ref(),
        &pod_gvk(),
        Some(ns.name()),
        ListOptions::default().labels("tier=a").limit(2),
    )
    .await;
    assert_eq!(names, ["a-1", "a-2", "a-3", "a-4", "a-5"]);
    assert_eq!(pages, 3);

    // Metadata carries what a list view needs: namespace, uid, version and labels.
    let page = port
        .list_metadata(
            &pod_gvk(),
            Some(ns.name()),
            &ListOptions::default().fields("metadata.name=b-1"),
        )
        .await
        .expect("field-selected metadata");
    assert_eq!(page.items.len(), 1);
    let meta = &page.items[0];
    assert_eq!(meta.namespace.as_deref(), Some(ns.name()));
    assert!(meta.uid.is_some() && meta.resource_version.is_some());
    assert_eq!(meta.labels.get("tier").map(|v| &**v), Some("b"));
    assert!(page.resource_version.is_some(), "a watch can start from it");

    // A cluster-scoped kind has no namespace.
    let page = port
        .list_metadata(
            &Gvk::new("", "v1", "Namespace"),
            None,
            &ListOptions::default().fields(format!("metadata.name={}", ns.name())),
        )
        .await
        .expect("namespace metadata");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].namespace, None);

    // A custom resource (the shared `Widget` fixtures, read only).
    let (names, _) = metadata_names(
        port.as_ref(),
        &Gvk::new("test.oxikube.dev", "v1", "Widget"),
        Some("oxikube-fixtures"),
        ListOptions::default(),
    )
    .await;
    for fixture in ["large", "medium", "small"] {
        assert!(names.iter().any(|n| n == fixture), "{fixture} in {names:?}");
    }
}

#[tokio::test]
async fn list_metadata_without_rights_is_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-meta-reader", vec![pods_only]).await;
    let context = ContextName::from("oxi-meta-reader");
    let client = kind
        .pool(kind.with_token_context(context.as_str(), &account.token))
        .get(&context)
        .await
        .expect("restricted client");
    let port = port(&client);

    wait_in(&ns, "the Role to allow listing pods", DEADLINE, || async {
        port.list_metadata(&pod_gvk(), Some(ns.name()), &ListOptions::default())
            .await
            .ok()
    })
    .await;
    let err = port
        .list_metadata(
            &Gvk::new("", "v1", "ConfigMap"),
            Some(ns.name()),
            &ListOptions::default(),
        )
        .await
        .expect_err("no rights on configmaps");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

/// What a consumer folding a feed holds: name to the last delivered object, plus every
/// `Applied` and `Deleted` name seen, in order.
#[derive(Default)]
struct Folded {
    objects: BTreeMap<String, Resource>,
    applied: Vec<String>,
    deleted: Vec<String>,
    restarts: usize,
}

impl Folded {
    /// Applies the next batch, waiting at most [`DEADLINE`] for it.
    async fn next(&mut self, feed: &mut WatchFeed) {
        let batch = tokio::time::timeout(DEADLINE, feed.next())
            .await
            .expect("a batch within the deadline")
            .expect("the feed is open")
            .expect("a batch, not an error");
        for delta in batch {
            match delta {
                Delta::Restarted(all) => {
                    self.restarts += 1;
                    self.objects = all.into_iter().map(|r| (r.name().to_owned(), r)).collect();
                }
                Delta::Applied(r) => {
                    self.applied.push(r.name().to_owned());
                    self.objects.insert(r.name().to_owned(), r);
                }
                Delta::Deleted(r) => {
                    self.deleted.push(r.name().to_owned());
                    self.objects.remove(r.name());
                }
            }
        }
    }

    /// Folds batches until `done` holds; fails the test after [`DEADLINE`].
    async fn until(&mut self, feed: &mut WatchFeed, what: &str, done: impl Fn(&Self) -> bool) {
        let fold = async {
            while !done(self) {
                self.next(feed).await;
            }
        };
        tokio::time::timeout(DEADLINE, fold)
            .await
            .unwrap_or_else(|_| panic!("{what} not seen within {DEADLINE:?}"));
    }

    fn names(&self) -> Vec<&str> {
        self.objects.keys().map(String::as_str).collect()
    }
}

/// A pending pod as the JSON body the port takes.
fn pod_body(name: &str, labels: &[(&str, &str)]) -> Value {
    serde_json::to_value(pending_pod(name, labels)).expect("pod json")
}

#[tokio::test]
async fn a_port_watch_folds_create_update_and_delete() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port = port(&client);
    let at = Some(ns.name());
    let write = WriteOptions::default();
    port.create(
        &pod_gvk(),
        at,
        &pod_body("before", &[("app", "watched")]),
        &write,
    )
    .await
    .expect("create before");
    port.create(
        &pod_gvk(),
        at,
        &pod_body("other", &[("app", "other")]),
        &write,
    )
    .await
    .expect("create other");

    let options = WatchOptions::default().labels("app=watched");
    let mut feed = port.watch(&pod_gvk(), at, &options).await.expect("watch");
    let mut folded = Folded::default();
    folded
        .until(&mut feed, "the initial list", |f| f.restarts > 0)
        .await;
    assert_eq!(folded.names(), ["before"], "the selector keeps `other` out");

    port.create(
        &pod_gvk(),
        at,
        &pod_body("during", &[("app", "watched")]),
        &write,
    )
    .await
    .expect("create during");
    let relabel = Patch::merge(json!({"metadata": {"labels": {"step": "two"}}}));
    port.patch(&pod_gvk(), at, "before", &relabel, &write)
        .await
        .expect("patch before");
    folded
        .until(&mut feed, "the create and the update", |f| {
            f.objects.contains_key("during")
                && f.objects
                    .get("before")
                    .and_then(|r| r.meta.labels.get("step"))
                    .is_some_and(|v| &**v == "two")
        })
        .await;

    let gone = DeleteOptions::default().grace_period_secs(0);
    port.delete(&pod_gvk(), at, "before", &gone)
        .await
        .expect("delete before");
    folded
        .until(&mut feed, "the delete", |f| {
            !f.objects.contains_key("before")
        })
        .await;
    assert_eq!(folded.names(), ["during"]);
    assert!(folded.applied.iter().any(|n| n == "during"));
    assert!(folded.deleted.iter().any(|n| n == "before"));
    assert!(folded.objects["during"].get("/spec/containers").is_some());
}

#[tokio::test]
async fn a_metadata_only_port_watch_carries_partial_objects() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port = port(&client);
    create_pods(
        &client,
        ns.name(),
        vec![pending_pod("slim", &[("app", "m")])],
    )
    .await;

    let options = WatchOptions::default().metadata_only();
    let mut feed = port
        .watch(&pod_gvk(), Some(ns.name()), &options)
        .await
        .expect("metadata watch");
    let mut folded = Folded::default();
    folded
        .until(&mut feed, "the initial list", |f| f.restarts > 0)
        .await;
    let slim = &folded.objects["slim"];
    assert!(slim.is_partial());
    assert_eq!(slim.meta.labels.get("app").map(|v| &**v), Some("m"));
    assert!(slim.get("/spec").is_none(), "no spec on a metadata feed");

    // `get` still returns the whole object.
    let full = port
        .get(&pod_gvk(), Some(ns.name()), "slim")
        .await
        .expect("get");
    assert!(!full.is_partial());
    assert!(full.get("/spec/containers").is_some());
}

/// The `policy/v1` eviction of `namespace/pod`, as `kubectl drain` posts it.
fn eviction(namespace: &str, pod: &str) -> Value {
    json!({
        "apiVersion": "policy/v1",
        "kind": "Eviction",
        "metadata": {"name": pod, "namespace": namespace},
        "deleteOptions": {"gracePeriodSeconds": 0},
    })
}

#[tokio::test]
async fn create_subresource_posts_an_eviction_and_classifies_failures() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port = port(&client);
    let at = Some(ns.name());
    let evict = Subresource::Other("eviction".into());
    create_pods(&client, ns.name(), vec![pending_pod("victim", &[])]).await;

    // Dry run: accepted, nothing happens.
    port.create_subresource(
        &pod_gvk(),
        at,
        "victim",
        &evict,
        &eviction(ns.name(), "victim"),
        &WriteOptions::dry_run(),
    )
    .await
    .expect("dry-run eviction");
    let still = port.get_opt(&pod_gvk(), at, "victim").await.expect("get");
    assert!(still.is_some_and(|p| !p.meta.is_terminating()));

    // For real: the pod goes.
    port.create_subresource(
        &pod_gvk(),
        at,
        "victim",
        &evict,
        &eviction(ns.name(), "victim"),
        &WriteOptions::default(),
    )
    .await
    .expect("eviction");
    wait_in(&ns, "the evicted pod to go", DEADLINE, || async {
        port.get_opt(&pod_gvk(), at, "victim")
            .await
            .expect("get")
            .is_none()
            .then_some(())
    })
    .await;

    // A missing pod is `NotFound`; a kind that does not serve the subresource is
    // `Unsupported`; a subresource name that is not a path segment never leaves the process.
    let err = port
        .create_subresource(
            &pod_gvk(),
            at,
            "victim",
            &evict,
            &eviction(ns.name(), "victim"),
            &WriteOptions::default(),
        )
        .await
        .expect_err("evicting a missing pod");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");

    let cm = Gvk::new("", "v1", "ConfigMap");
    port.create(
        &cm,
        at,
        &json!({"metadata": {"name": "plain"}}),
        &WriteOptions::default(),
    )
    .await
    .expect("create configmap");
    let err = port
        .create_subresource(
            &cm,
            at,
            "plain",
            &evict,
            &json!({}),
            &WriteOptions::default(),
        )
        .await
        .expect_err("configmaps serve no eviction");
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");

    let err = port
        .create_subresource(
            &pod_gvk(),
            at,
            "victim",
            &Subresource::Other("eviction/../status".into()),
            &json!({}),
            &WriteOptions::default(),
        )
        .await
        .expect_err("a path in the subresource name");
    assert_eq!(err.kind(), ErrorKind::Validation);
}
