//! Namespace scope changes: kept namespaces keep their feed, new ones are seeded, the
//! subscription (filter, sort) survives.

use std::time::Duration;

use oxikube_domain::session::WatchScope;
use oxikube_ports::Delta;
use oxikube_testkit::{ResourceCall, Timeline};

use super::*;
use crate::store::{FeedScope, FeedState, SortField, SortKey};

fn watched_namespaces(h: &Harness) -> Vec<Option<String>> {
    h.resources
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            ResourceCall::Watch { namespace, .. } => Some(namespace),
            _ => None,
        })
        .collect()
}

#[test]
fn a_namespace_change_keeps_the_feeds_that_stay_and_releases_the_rest() {
    let mut h = Harness::with_options(options_with_grace(0));
    for (ns, name) in [("a", "1"), ("b", "2"), ("c", "3")] {
        h.resources.insert(p(ns, name, "1"));
    }
    let mut sub = h.subscribe(
        in_namespaces(pods(), &["a", "b"]).with_sort(SortKey::by(SortField::Name).descending()),
    );
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["b/2", "a/1"]);
    assert_eq!(watched_namespaces(&h), [Some("a".into()), Some("b".into())]);

    sub.rescope(WatchScope::Namespaces(vec!["b".into(), "c".into()]));
    h.settle();
    assert_eq!(
        watched_namespaces(&h),
        [Some("a".into()), Some("b".into()), Some("c".into())],
        "only c starts; b is untouched"
    );
    assert_eq!(h.resources.live_watches(), 2, "a was released");
    let keys: Vec<FeedScope> = h.store.feeds().into_iter().map(|f| f.key.scope).collect();
    assert_eq!(
        keys,
        [
            FeedScope::Namespace("b".into()),
            FeedScope::Namespace("c".into())
        ]
    );

    m.drain(&mut sub);
    assert_eq!(m.names(), ["c/3", "b/2"], "same subscription, same sort");
    assert!(matches!(m.last_rows(), RowChange::Snapshot(_)));
    assert_eq!(sub.state(), FeedState::Ready);
    assert_eq!(
        sub.query().scope,
        WatchScope::Namespaces(vec!["b".into(), "c".into()])
    );
}

#[test]
fn narrowing_from_all_namespaces_shows_the_cached_rows_before_the_new_feed_lists() {
    let mut h = Harness::with_options(options_with_grace(0));
    h.resources.insert(p("a", "1", "1"));
    h.resources.insert(p("b", "2", "1"));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["a/1", "b/2"]);

    // The namespaced feed takes 5 s to list, and by then a/1 changed and a/3 appeared.
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::from_secs(5),
                batch(vec![Delta::Restarted(vec![
                    p("a", "1", "2"),
                    p("a", "3", "1"),
                ])]),
            )
            .keep_open(),
    );
    sub.rescope(WatchScope::Namespaces(vec!["a".into()]));
    h.settle();
    m.drain(&mut sub);
    assert_eq!(
        m.names(),
        ["a/1"],
        "seeded from the cluster-wide cache at once"
    );
    assert_eq!(sub.state(), FeedState::Warming);
    assert_eq!(
        h.resources.live_watches(),
        1,
        "the cluster-wide feed stopped"
    );

    h.advance(5);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["a/1", "a/3"]);
    assert_eq!(m.rows[0].meta().resource_version.as_deref(), Some("2"));
    assert_eq!(sub.state(), FeedState::Ready);
}

#[test]
fn widening_back_to_all_namespaces_starts_one_cluster_feed() {
    let mut h = Harness::with_options(options_with_grace(0));
    h.resources.insert(p("a", "1", "1"));
    h.resources.insert(p("b", "2", "1"));
    let mut sub = h.subscribe(in_namespaces(pods(), &["a"]));
    sub.rescope(WatchScope::Cluster);
    h.settle();
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["a/1", "b/2"]);
    assert_eq!(watched_namespaces(&h), [Some("a".into()), None]);
    assert_eq!(h.resources.live_watches(), 1);

    sub.rescope(WatchScope::Cluster);
    h.settle();
    assert_eq!(
        watched_namespaces(&h).len(),
        2,
        "same scope: nothing restarts"
    );
}
