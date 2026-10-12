//! The Namespace kind over fake feeds (E12-S16): per-namespace counts read off open feeds, the
//! delete advice built from the cached namespaces and counts, and the Pods column.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::NoteLevel;
use oxikube_ports::Delta;
use oxikube_testkit::deployment;
use serde_json::json;

use super::*;
use crate::columns::{ColumnId, ColumnProvider, CoreColumns};
use crate::namespaces::{NamespaceColumns, PODS_COLUMN, delete_advice};
use crate::store::{FeedScope, StoreObject};

fn namespaces() -> Gvk {
    Gvk::new("", "v1", "Namespace")
}

fn ns(name: &str, extra: serde_json::Value) -> Resource {
    let mut value = json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
        "spec": { "finalizers": ["kubernetes"] },
        "status": { "phase": "Active" },
    });
    if let Some(extra) = extra.as_object() {
        for (key, v) in extra {
            value["metadata"][key] = v.clone();
        }
    }
    Resource::from_json(value).expect("namespace")
}

fn pod_in(ns: &str, name: &str) -> Resource {
    p(ns, name, "1")
}

#[test]
fn a_namespace_count_is_read_off_the_open_feed_and_follows_events() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            pod_in("x", "a"),
            pod_in("x", "b"),
            pod_in("y", "c"),
        ])]),
        batch(vec![
            Delta::Applied(pod_in("y", "d")),
            Delta::Deleted(pod_in("x", "a")),
        ]),
    ]));
    assert_eq!(
        h.store.count_in_namespace(&pods(), "x"),
        None,
        "nothing watches pods yet: unknown, not zero"
    );
    let _table = h.subscribe(all(pods()));
    assert_eq!(h.store.count_in_namespace(&pods(), "x"), Some(2));
    assert_eq!(h.store.count_in_namespace(&pods(), "y"), Some(1));
    assert_eq!(
        h.store.count_in_namespace(&pods(), "empty"),
        Some(0),
        "a listed feed that has no pod there knows it is zero"
    );
    h.advance(1);
    assert_eq!(h.store.count_in_namespace(&pods(), "x"), Some(1));
    assert_eq!(h.store.count_in_namespace(&pods(), "y"), Some(2));
    assert_eq!(h.resources.live_watches(), 1, "reading started nothing");
}

#[test]
fn a_namespace_scoped_feed_only_knows_its_own_namespace() {
    let mut h = Harness::with_objects([pod_in("x", "a"), pod_in("y", "b")]);
    let _table = h.subscribe(in_namespaces(pods(), &["x"]));
    assert_eq!(h.store.count_in_namespace(&pods(), "x"), Some(1));
    assert_eq!(
        h.store.count_in_namespace(&pods(), "y"),
        None,
        "no open feed covers y"
    );
}

#[test]
fn a_namespaces_objects_and_their_revision_are_read_off_the_open_feed() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            pod_in("x", "a"),
            pod_in("x", "b"),
            pod_in("y", "c"),
        ])]),
        batch(vec![Delta::Applied(pod_in("y", "d"))]),
        batch(vec![Delta::Deleted(pod_in("x", "a"))]),
    ]));
    assert!(
        h.store.namespace_objects(&pods(), "x").is_none(),
        "nothing watches pods yet: unknown, not an empty namespace"
    );
    assert_eq!(h.store.namespace_revision(&pods(), "x"), None);
    let _table = h.subscribe(all(pods()));
    let names = |h: &Harness, ns: &str| {
        let mut names: Vec<String> = h
            .store
            .namespace_objects(&pods(), ns)
            .expect("listed")
            .iter()
            .map(|o| o.name().to_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&h, "x"), ["a", "b"]);
    assert_eq!(names(&h, "y"), ["c"]);
    assert!(
        names(&h, "empty").is_empty(),
        "a listed feed knows the namespace is empty"
    );

    // Any change of the feed moves the revision; a read does not.
    let listed = h.store.namespace_revision(&pods(), "x").expect("listed");
    assert_eq!(h.store.namespace_revision(&pods(), "x"), Some(listed));
    h.advance(1);
    let after_apply = h.store.namespace_revision(&pods(), "x").unwrap();
    assert!(after_apply > listed, "a pod of y arrived");
    assert_eq!(names(&h, "y"), ["c", "d"]);
    h.advance(1);
    assert!(h.store.namespace_revision(&pods(), "x").unwrap() > after_apply);
    assert_eq!(names(&h, "x"), ["b"]);
    assert_eq!(h.resources.live_watches(), 1, "reading started nothing");
}

#[test]
fn a_namespace_scoped_feed_serves_its_own_namespace_objects_only() {
    let mut h = Harness::with_objects([pod_in("x", "a"), pod_in("y", "b")]);
    let _table = h.subscribe(in_namespaces(pods(), &["x"]));
    assert_eq!(
        h.store.namespace_objects(&pods(), "x").map(|o| o.len()),
        Some(1)
    );
    assert!(h.store.namespace_objects(&pods(), "y").is_none());
    assert!(h.store.namespace_revision(&pods(), "x").is_some());
    assert!(h.store.namespace_revision(&pods(), "y").is_none());
}

#[test]
fn cluster_scoped_objects_are_read_by_name() {
    let mut h = Harness::with_objects([ns("a", json!({})), ns("b", json!({}))]);
    assert!(h.store.cluster_object(&namespaces(), "a").is_none());
    let _table = h.subscribe(all(namespaces()));
    let found = h.store.cluster_object(&namespaces(), "a").expect("cached");
    assert_eq!(found.name(), "a");
    assert!(h.store.cluster_object(&namespaces(), "missing").is_none());
    let mut names: Vec<String> = h
        .store
        .cluster_objects(&namespaces())
        .expect("listed")
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["a", "b"]);
    let _ = FeedScope::Cluster;
}

#[test]
fn advice_counts_what_the_store_holds_and_leaves_out_what_it_does_not() {
    let mut h = Harness::with_objects([
        ns("team-a", json!({})),
        pod_in("team-a", "p1"),
        pod_in("team-a", "p2"),
        pod_in("other", "p3"),
    ]);
    let _namespaces = h.subscribe(all(namespaces()));
    let _pods = h.subscribe(all(pods()));

    let advice = delete_advice(Some(&h.store), "team-a");
    assert_eq!(advice.contents.get("Pod"), Some(&2));
    assert!(
        !advice.contents.contains_key("Deployment"),
        "deployments are not watched: absent, not zero"
    );
    let text: Vec<&str> = advice.notes.iter().map(|n| n.text.as_str()).collect();
    assert!(
        text.iter().any(|t| t.contains("2 Pods")),
        "the dialog says what goes: {text:?}"
    );

    // Once deployments are watched too they are counted.
    h.resources
        .insert(deployment().namespace("team-a").name("api").build());
    let _deployments = h.subscribe(all(Gvk::new("apps", "v1", "Deployment")));
    let advice = delete_advice(Some(&h.store), "team-a");
    assert_eq!(advice.contents.get("Deployment"), Some(&1));
    assert!(
        advice
            .notes
            .iter()
            .any(|n| n.text.contains("2 Pods, 1 Deployment")),
        "pods first, then deployments: {:?}",
        advice.notes
    );
}

#[test]
fn advice_warns_about_system_namespaces_even_without_a_store() {
    let advice = delete_advice(None, "kube-system");
    assert_eq!(advice.notes.len(), 1);
    assert_eq!(advice.notes[0].level, NoteLevel::Danger);
    assert!(advice.contents.is_empty());
    assert!(delete_advice(None, "team-a").notes.is_empty());
}

#[test]
fn advice_finds_hnc_children_and_the_parent_of_a_subnamespace() {
    let mut h = Harness::with_objects([
        ns(
            "team-a",
            json!({ "labels": { "team-a.tree.hnc.x-k8s.io/depth": "0" } }),
        ),
        ns(
            "team-a-dev",
            json!({
                "annotations": { "hnc.x-k8s.io/subnamespace-of": "team-a" },
                "labels": {
                    "team-a.tree.hnc.x-k8s.io/depth": "1",
                    "team-a-dev.tree.hnc.x-k8s.io/depth": "0",
                },
            }),
        ),
        ns(
            "team-a-prod",
            json!({ "labels": { "team-a.tree.hnc.x-k8s.io/depth": "1" } }),
        ),
        ns("unrelated", json!({})),
    ]);
    let _namespaces = h.subscribe(all(namespaces()));

    let parent = delete_advice(Some(&h.store), "team-a");
    assert!(
        parent
            .notes
            .iter()
            .any(|n| n.text.contains("2 HNC subnamespaces sit below")),
        "{:?}",
        parent.notes
    );
    let child = delete_advice(Some(&h.store), "team-a-dev");
    assert!(
        child
            .notes
            .iter()
            .any(|n| n.text.contains("subnamespace of team-a")),
        "{:?}",
        child.notes
    );
    assert!(delete_advice(Some(&h.store), "unrelated").notes.is_empty());
}

#[test]
fn the_pods_column_sits_after_status_and_reads_live_counts() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            pod_in("team-a", "p1"),
            pod_in("team-a", "p2"),
        ])]),
        batch(vec![Delta::Applied(pod_in("team-a", "p3"))]),
    ]));
    let provider = NamespaceColumns::new(Arc::new(CoreColumns::new()), Some(h.store.clone()));
    h.settle();
    let columns = provider.columns(&namespaces(), Capabilities::empty());
    let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["name", "status", PODS_COLUMN, "age", "labels"]);

    let object = StoreObject::Resource(ns("team-a", json!({})));
    let cell = |provider: &NamespaceColumns| {
        provider
            .cell(&object, &ColumnId::new(PODS_COLUMN), Timestamp::now())
            .display()
            .to_owned()
    };
    // The provider leased the pods feed itself: nobody else is watching pods.
    assert_eq!(cell(&provider), "2");
    h.advance(1);
    assert_eq!(cell(&provider), "3", "the count follows pod events");

    // The other columns are the core ones.
    let name = provider.cell(&object, &ColumnId::new("name"), Timestamp::now());
    assert_eq!(name.display(), "team-a");
}

#[test]
fn the_pods_column_is_a_dash_when_no_feed_covers_the_namespace() {
    let provider = NamespaceColumns::new(Arc::new(CoreColumns::new()), None);
    let object = StoreObject::Resource(ns("team-a", json!({})));
    let cell = provider.cell(&object, &ColumnId::new(PODS_COLUMN), Timestamp::now());
    assert_eq!(cell.display(), "–");
    assert!(cell.is_blank() || cell.sort() == crate::columns::CellSort::None);
}

#[test]
fn dropping_the_provider_releases_its_pod_feed() {
    let mut h = Harness::with_objects([pod_in("x", "a")]);
    let provider = NamespaceColumns::new(Arc::new(CoreColumns::new()), Some(h.store.clone()));
    h.settle();
    assert_eq!(
        h.resources.live_watches(),
        2,
        "the provider leases pods and deployments"
    );
    drop(provider);
    h.advance(120);
    assert_eq!(
        h.resources.live_watches(),
        0,
        "the grace timer stopped the feeds"
    );
}

#[test]
fn the_pods_column_is_a_dash_not_a_zero_when_the_pods_feed_is_forbidden() {
    let mut h = Harness::with_objects([pod_in("team-a", "p1")]);
    h.resources
        .script()
        .watch
        .push_err(oxikube_domain::OxiError::forbidden("pods is forbidden"));
    let provider = NamespaceColumns::new(Arc::new(CoreColumns::new()), Some(h.store.clone()));
    h.settle();
    let object = StoreObject::Resource(ns("team-a", json!({})));
    let cell = provider.cell(&object, &ColumnId::new(PODS_COLUMN), Timestamp::now());
    assert_eq!(cell.display(), "–", "no access is not zero pods");
    assert_eq!(provider.pods_in("team-a"), None);
}

/// The pod count is an index lookup, not a scan: with 10 000 pods in 100 namespaces a lookup
/// costs about what it costs with ten, so a table of 100 namespaces reads its Pods column for
/// the price of 100 hash lookups per frame. (The bound is generous so a loaded CI machine does
/// not fail it; a scan of 10 000 pods per row would take seconds.)
#[test]
fn a_pod_count_costs_a_lookup_not_a_scan_at_ten_thousand_pods() {
    let pods: Vec<Resource> = (0..10_000)
        .map(|i| p(&format!("ns-{}", i % 100), &format!("pod-{i}"), "1"))
        .collect();
    let mut h = Harness::with_objects(pods);
    let _table = h.subscribe(all(pods_gvk()));
    assert_eq!(h.store.count_in_namespace(&pods_gvk(), "ns-7"), Some(100));

    let started = std::time::Instant::now();
    let mut total = 0usize;
    for round in 0..100 {
        for ns in 0..100 {
            total += h
                .store
                .count_in_namespace(&pods_gvk(), &format!("ns-{}", (ns + round) % 100))
                .unwrap_or(0);
        }
    }
    let elapsed = started.elapsed();
    assert_eq!(total, 100 * 10_000);
    eprintln!(
        "10 000 pod counts over 10 000 pods: {elapsed:?} ({:?} per lookup)",
        elapsed / 10_000
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "10 000 lookups took {elapsed:?}"
    );
}

fn pods_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}
