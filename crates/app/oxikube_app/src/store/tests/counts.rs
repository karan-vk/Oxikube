//! Counts over fake feeds (E07-S11): they follow adds, updates and deletes, a forbidden kind is
//! "no access" and not zero, a refused feed degrades without starting anything, and counting
//! alone never opens a feed.

use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
use oxikube_testkit::{Timeline, deployment, pod};
use serde_json::json;

use super::*;
use crate::store::{CountState, CountTarget, KindCount, MaxFeeds};

fn pod_target() -> CountTarget {
    CountTarget::core("", "pods").expect("pods are a core target")
}

fn deployment_target() -> CountTarget {
    CountTarget::core("apps", "deployments").expect("deployments are a core target")
}

/// A pod `ns/name` with a phase.
fn phased(ns: &str, name: &str, phase: &str) -> Resource {
    let builder = pod().namespace(ns).name(name);
    let builder = match phase {
        "Running" => builder.running(),
        "Succeeded" => builder.succeeded(),
        "Failed" => builder.failed(),
        _ => builder.pending(),
    };
    builder.build()
}

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

#[test]
fn counts_follow_adds_updates_and_deletes() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            phased("x", "a", "Running"),
            phased("x", "b", "Pending"),
            phased("y", "c", "Succeeded"),
        ])]),
        batch(vec![Delta::Applied(phased("x", "b", "Running"))]),
        batch(vec![
            Delta::Applied(phased("y", "d", "Failed")),
            Delta::Deleted(phased("x", "a", "Running")),
        ]),
    ]));
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(lease.counts(), [counted(3, 3, 2)], "one pod is pending");
    h.advance(1);
    assert_eq!(
        lease.counts(),
        [counted(3, 3, 3)],
        "an update turns it healthy"
    );
    h.advance(1);
    assert_eq!(
        lease.counts(),
        [counted(3, 3, 2)],
        "one added (failed), one deleted"
    );
}

#[test]
fn counting_alone_starts_no_feed() {
    let mut h = Harness::with_objects([p("x", "a", "1")]);
    let store = h.store.clone();
    let counts = store.counts(
        &[pod_target(), deployment_target()],
        &NamespaceSelection::All,
    );
    h.settle();
    assert_eq!(counts, [CountState::NotWatched, CountState::NotWatched]);
    assert!(h.store.feeds().is_empty(), "no cache entry was created");
    assert_eq!(h.resources.live_watches(), 0);
    assert!(h.resources.recorded_calls().is_empty(), "no port call");
}

#[test]
fn an_open_feed_is_reused_and_no_second_one_starts() {
    let mut h = Harness::with_objects([phased("x", "a", "Running"), phased("y", "b", "Pending")]);
    let _table = h.subscribe(all(pods()));
    assert_eq!(h.resources.live_watches(), 1);
    let all_ns = h.store.count(&pod_target(), &NamespaceSelection::All);
    assert_eq!(all_ns, counted(2, 2, 1));
    // A namespace selection reads the cluster-wide feed's objects of that namespace.
    let one = h
        .store
        .count(&pod_target(), &NamespaceSelection::single("x"));
    assert_eq!(one, counted(1, 1, 1));
    assert_eq!(h.store.feeds().len(), 1);
    assert_eq!(h.resources.live_watches(), 1, "counting reused the feed");
}

#[test]
fn a_forbidden_kind_is_no_access_not_zero() {
    let mut h = Harness::new();
    h.resources
        .script()
        .watch
        .push_err(OxiError::forbidden("pods is forbidden"));
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    let [state] = lease.counts().try_into().expect("one count");
    assert!(state.is_no_access(), "{state:?}");
    assert_eq!(state.count(), None);
}

#[test]
fn an_over_budget_kind_degrades_and_starts_nothing() {
    let budget = Arc::new(MaxFeeds::new(1));
    let mut h = Harness::with_options(StoreOptions {
        budget,
        ..options_with_grace(30)
    });
    h.resources.insert(phased("x", "a", "Running"));
    let lease = h.store.lease_counts(
        vec![pod_target(), deployment_target()],
        &NamespaceSelection::All,
    );
    h.settle();
    let counts = lease.counts();
    assert_eq!(counts[0], counted(1, 1, 1), "the first feed fits");
    assert!(
        matches!(&counts[1], CountState::OverBudget { message } if message.contains("limit 1")),
        "{:?}",
        counts[1]
    );
    assert_eq!(
        h.resources.live_watches(),
        1,
        "the refused kind never opened"
    );
}

#[test]
fn a_feed_that_has_not_listed_yet_is_loading() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                std::time::Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![phased("x", "a", "Running")])]),
            )
            .keep_open(),
    );
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(lease.counts(), [CountState::Loading]);
    h.advance(5);
    assert_eq!(lease.counts(), [counted(1, 1, 1)]);
}

#[test]
fn a_lease_follows_the_namespace_selection() {
    let mut h = Harness::with_options(options_with_grace(0));
    for pod in [
        phased("a", "p1", "Running"),
        phased("a", "p2", "Pending"),
        phased("b", "p3", "Running"),
    ] {
        h.resources.insert(pod);
    }
    let mut lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::single("a"));
    h.settle();
    assert_eq!(lease.counts(), [counted(2, 2, 1)]);
    lease.rescope(&NamespaceSelection::from_names(["a", "b"]));
    h.settle();
    assert_eq!(lease.counts(), [counted(3, 3, 2)]);
    lease.rescope(&NamespaceSelection::single("b"));
    h.settle();
    assert_eq!(lease.counts(), [counted(1, 1, 1)]);
    assert_eq!(h.resources.live_watches(), 1, "one namespace, one feed");
}

#[test]
fn dropping_the_lease_releases_its_feeds_after_the_grace_period() {
    let mut h = Harness::with_options(options_with_grace(10));
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(h.resources.live_watches(), 1);
    drop(lease);
    h.advance(5);
    assert_eq!(h.resources.live_watches(), 1, "still in its grace period");
    h.advance(6);
    assert_eq!(h.resources.live_watches(), 0);
}

#[test]
fn a_lease_builds_no_row_index() {
    let mut h = Harness::new();
    let many: Vec<Resource> = (0..300)
        .map(|i| phased("x", &format!("p{i}"), "Running"))
        .collect();
    h.resources
        .script()
        .watch
        .push_ok(timeline(vec![batch(vec![Delta::Restarted(many)])]));
    let lease = h
        .store
        .lease_counts(vec![pod_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(lease.counts(), [counted(300, 300, 300)]);
    let mut table = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut table);
    assert_eq!(m.rows.len(), 300, "a table beside a lease sees every row");
}

#[test]
fn workload_health_counts_ready_against_desired() {
    let mut h = Harness::with_objects([
        deployment()
            .namespace("x")
            .name("ok")
            .replicas(2)
            .ready(2)
            .build(),
        deployment()
            .namespace("x")
            .name("short")
            .replicas(3)
            .ready(1)
            .build(),
    ]);
    let lease = h
        .store
        .lease_counts(vec![deployment_target()], &NamespaceSelection::All);
    h.settle();
    assert_eq!(lease.counts(), [counted(2, 2, 1)]);
}

#[test]
fn table_kinds_count_without_health() {
    let mut h = Harness::new();
    let columns: Arc<[TableColumn]> = Arc::from(vec![TableColumn {
        name: "Name".into(),
        column_type: "string".into(),
        ..TableColumn::default()
    }]);
    let mut meta = oxikube_domain::ObjectMeta::named("w");
    meta.namespace = Some("x".into());
    meta.resource_version = Some("1".into());
    h.tables.script().table_feed.push_ok(
        Timeline::immediate([TableBatch {
            columns: Some(columns),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![TableRow {
                cells: vec![json!("w")],
                meta: Some(meta),
                object: None,
            }])]),
            source: TableSource::Server,
        }])
        .keep_open(),
    );
    let widget = CountTarget::new(widgets(), Scope::Namespaced);
    let lease = h.store.lease_counts(vec![widget], &NamespaceSelection::All);
    h.settle();
    let [state] = lease.counts().try_into().expect("one count");
    let count = state.count().expect("counted");
    assert_eq!((count.total, count.has_health()), (1, false));
    assert!(count.all_healthy());
}

#[test]
fn only_the_overview_kinds_count_eagerly() {
    let h = Harness::new();
    assert!(h.store.counts_eagerly(&pods()));
    assert!(
        h.store
            .counts_eagerly(&Gvk::new("apps", "v1", "Deployment"))
    );
    assert!(!h.store.counts_eagerly(&Gvk::new("", "v1", "ConfigMap")));
    assert!(!h.store.counts_eagerly(&widgets()));
}
