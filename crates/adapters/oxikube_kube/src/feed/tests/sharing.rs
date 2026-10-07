//! One copy per object (#508): the reflector store and the consumer hold the same JSON tree,
//! so 10 000 pods are not kept twice (the store's cache would otherwise hold a second
//! `serde_json::Value` tree of every object, about 16 KB per small pod).

use std::sync::Arc;

use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, WatchOptions};

use super::server::*;
use super::*;
use crate::feed::RelistDelivery;

async fn open(server: &FeedServer, config: FeedConfig) -> ReflectorFeed {
    server
        .resources(config)
        .reflector_feed(
            &pod_gvk(),
            &WatchScope::Namespaces(vec!["default".into()]),
            &WatchOptions::default(),
        )
        .await
        .unwrap()
}

/// Every resource `batch` delivers, whatever the delta.
fn delivered(batch: DeltaBatch<Resource>) -> Vec<Resource> {
    batch
        .into_iter()
        .flat_map(|delta| match delta {
            Delta::Applied(r) | Delta::Deleted(r) => vec![r],
            Delta::Restarted(all) => all,
        })
        .collect()
}

/// Asserts that each of `resources` holds the very JSON tree the reflector store holds.
fn assert_shared(feed: &ReflectorFeed, resources: &[Resource]) {
    let stored = feed.snapshot();
    assert!(!resources.is_empty());
    for resource in resources {
        let entry = stored
            .iter()
            .find(|o| o.name() == resource.name())
            .unwrap_or_else(|| panic!("{} is in the reflector store", resource.name()));
        assert!(
            Arc::ptr_eq(&entry.json, &resource.json),
            "{} was copied on its way to the consumer",
            resource.name()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_list_and_live_changes_share_the_stores_objects() {
    let server = FeedServer::new(31);
    server.list(
        PODS,
        pod_list(vec![pod("a", "ua", "1"), pod("b", "ub", "2")], "10"),
    );
    server.watch(
        PODS,
        Reply::Events(vec![
            event("ADDED", pod("c", "uc", "12")),
            event("ADDED", pod("d", "ud", "13")),
        ]),
    );
    let mut feed = open(&server, config()).await;

    // The live changes touch other pods, so the store still holds the listed `a` and `b`.
    let opening = delivered(next_batch(&mut feed).await);
    assert_eq!(opening.len(), 2);
    assert_shared(&feed, &opening);

    let live = delivered(next_batch(&mut feed).await);
    assert_eq!(live.len(), 2);
    assert_shared(&feed, &live);
}

#[tokio::test(start_paused = true)]
async fn a_relist_shares_the_stores_objects_as_a_diff_and_as_a_snapshot() {
    for relist in [RelistDelivery::Diff, RelistDelivery::Snapshot] {
        let server = FeedServer::new(31);
        server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
        server.list(
            PODS,
            pod_list(vec![pod("a", "ua", "1"), pod("d", "ud", "21")], "30"),
        );
        server.watch(PODS, Reply::Events(vec![error_event(410, "Expired")]));
        let config = FeedConfig { relist, ..config() };
        let mut feed = open(&server, config).await;

        next_batch(&mut feed).await;
        assert!(next(&mut feed).await.is_err(), "410 Gone");
        let relisted = delivered(next_batch(&mut feed).await);
        assert!(
            relisted.iter().any(|r| r.name() == "d"),
            "{relist:?}: {relisted:?}"
        );
        assert_shared(&feed, &relisted);
    }
}
