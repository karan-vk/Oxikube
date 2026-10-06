//! Namespace selections: one feed per selected namespace, reuse across selection changes.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Scope;
use oxikube_domain::session::{NamespaceSelection, WatchScope};
use oxikube_ports::FeedVariant;

use super::*;
use crate::budget::ScopeChange;

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

fn template() -> FeedRequest {
    FeedRequest::new(pods(), FeedVariant::Full)
}

fn ns(names: &[&str]) -> Vec<Option<String>> {
    names.iter().map(|n| Some((*n).to_owned())).collect()
}

#[tokio::test(start_paused = true)]
async fn switching_from_a_b_to_b_c_stops_a_keeps_b_and_starts_c() {
    let (registry, source) = registry(roomy());
    let mut lease = registry
        .subscribe_selection(template(), Scope::Namespaced, &set(&["a", "b"]))
        .await
        .unwrap();
    assert_eq!(source.opened(), vec![full(pods(), "a"), full(pods(), "b")]);
    let streams = lease.take_feeds();
    assert_eq!(
        streams.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        ns(&["a", "b"])
    );
    assert_eq!(
        lease.scope(),
        WatchScope::Namespaces(vec!["a".into(), "b".into()])
    );

    let change = lease.reselect(&set(&["b", "c"])).await.unwrap();
    assert_eq!(
        change,
        ScopeChange {
            start: ns(&["c"]),
            keep: ns(&["b"]),
            stop: ns(&["a"]),
        }
    );
    assert_eq!(
        source.opened(),
        vec![full(pods(), "a"), full(pods(), "b"), full(pods(), "c")],
        "b is not reopened"
    );
    let more = lease.take_feeds();
    assert_eq!(more.len(), 1, "only c's stream is new");
    assert_eq!(
        lease.scope(),
        WatchScope::Namespaces(vec!["b".into(), "c".into()])
    );

    // `a` idles for the grace period, then stops; `b` and `c` stay.
    let (a, b, c) = (
        source.feed(&full(pods(), "a")),
        source.feed(&full(pods(), "b")),
        source.feed(&full(pods(), "c")),
    );
    assert_eq!(registry.stats().idle_feeds, 1);
    tokio::time::sleep(Duration::from_secs(31)).await;
    settle().await;
    assert!(!a.is_alive());
    assert!(b.is_alive() && c.is_alive());
    assert_eq!(registry.stats().feeds, 2);
    drop((streams, more));
}

#[tokio::test(start_paused = true)]
async fn switching_back_within_the_grace_period_reopens_nothing() {
    let (registry, source) = registry(roomy());
    let mut lease = registry
        .subscribe_selection(template(), Scope::Namespaced, &set(&["a"]))
        .await
        .unwrap();
    let _streams = lease.take_feeds();
    lease.reselect(&set(&["b"])).await.unwrap();
    let _more = lease.take_feeds();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let change = lease.reselect(&set(&["a"])).await.unwrap();
    assert_eq!(change.start, ns(&["a"]));
    assert_eq!(source.opened(), vec![full(pods(), "a"), full(pods(), "b")]);
    assert!(
        lease.take_feeds().is_empty(),
        "a's stream is still with its consumer"
    );
}

#[tokio::test(start_paused = true)]
async fn all_is_one_cluster_wide_feed_and_cluster_kinds_ignore_a_set() {
    let (registry, source) = registry(roomy());
    let mut lease = registry
        .subscribe_selection(template(), Scope::Namespaced, &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(lease.scope(), WatchScope::Cluster);
    let change = lease.reselect(&set(&["a"])).await.unwrap();
    assert_eq!((change.start, change.stop), (ns(&["a"]), vec![None]));
    let unchanged = lease.reselect(&set(&["a"])).await.unwrap();
    assert!(unchanged.is_empty());

    let nodes = FeedRequest::new(Gvk::new("", "v1", "Node"), FeedVariant::Full);
    let node_lease = registry
        .subscribe_selection(nodes.clone(), Scope::Cluster, &set(&["a", "b"]))
        .await
        .unwrap();
    assert_eq!(node_lease.scope(), WatchScope::Cluster);
    assert_eq!(source.opened().last(), Some(&nodes));
}

#[tokio::test(start_paused = true)]
async fn a_refused_namespace_rolls_the_selection_back() {
    let config = BudgetConfig {
        max_feeds: 2,
        ..roomy()
    };
    let (registry, _source) = registry(config);
    let mut lease = registry
        .subscribe_selection(template(), Scope::Namespaced, &set(&["a", "b"]))
        .await
        .unwrap();
    let _streams = lease.take_feeds();

    // a and b are released first, so c and d fit by evicting them; e does not.
    let err = lease.reselect(&set(&["c", "d", "e"])).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert_eq!(
        lease.scope(),
        WatchScope::Namespaces(vec!["a".into(), "b".into()])
    );
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.subscribers), (2, 2));
}

/// The resource store re-scopes from a task on the runtime (`spawn_kube`): the future of
/// `reselect` must be `Send` although the leases hold feed streams, which are not `Sync`.
#[tokio::test(start_paused = true)]
async fn reselecting_runs_on_a_spawned_task() {
    let (registry, _source) = registry(roomy());
    let mut lease = registry
        .subscribe_selection(template(), Scope::Namespaced, &set(&["a"]))
        .await
        .unwrap();
    let lease = tokio::spawn(async move {
        lease.reselect(&set(&["b"])).await.unwrap();
        lease
    })
    .await
    .unwrap();
    assert_eq!(lease.scope(), WatchScope::Namespaces(vec!["b".into()]));
}
