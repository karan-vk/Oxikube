//! [`BudgetedResources`] over the in-process fake API (E04-F543): `watch` and `table_feed` are
//! owned feeds of the registry, every other read and every write goes straight to the client.

use oxikube_domain::ErrorKind;
use oxikube_ports::{
    FeedVariant, ListOptions, ResourceReader, TableFeedPort, TableOptions, WatchOptions,
};

use super::kube::{PODS, pod_item, pod_list, resources, server};
use super::*;
use crate::budget::BudgetedResources;

fn budgeted(api: &crate::fake_api::FakeApi, config: BudgetConfig) -> BudgetedResources {
    let resources = resources(api);
    let registry = FeedRegistry::for_resources(cluster(), resources.clone(), config);
    BudgetedResources::new(resources, registry)
}

#[tokio::test]
async fn a_watch_is_a_counted_feed_of_the_registry_until_its_stream_drops() {
    let api = server();
    api.reply(PODS, 200, pod_list(vec![pod_item("a", "1")]));
    let ports = budgeted(&api, roomy());
    let options = WatchOptions::default().labels("app=web");
    let mut feed = ports
        .watch(&pods(), Some("default"), &options)
        .await
        .unwrap();
    let first = next(&mut feed).await.unwrap().unwrap();
    assert!(first.contains_restart());

    let stats = ports.feeds().stats();
    assert_eq!((stats.feeds, stats.objects), (1, 1));
    assert!(stats.bytes > 0, "counted over the byte-counting client");
    let stat = &stats.per_feed[0];
    assert_eq!(stat.namespace.as_deref(), Some("default"));
    assert_eq!(stat.variant, FeedVariant::Full);
    let list = api
        .requests()
        .into_iter()
        .find(|r| r.path == PODS)
        .expect("the list");
    assert!(
        list.query.contains("labelSelector=app%3Dweb"),
        "{}",
        list.query
    );

    drop(feed);
    assert_eq!(ports.feeds().stats().feeds, 0, "torn down with its stream");
}

#[tokio::test]
async fn a_metadata_watch_is_a_metadata_feed_and_a_full_one_is_never_degraded() {
    let api = server();
    api.reply(PODS, 200, pod_list(vec![]));
    let config = BudgetConfig {
        metadata_above: 0,
        ..roomy()
    };
    let ports = budgeted(&api, config);
    let _full = ports
        .watch(&pods(), Some("default"), &WatchOptions::default())
        .await
        .unwrap();
    let _meta = ports
        .watch(
            &pods(),
            Some("default"),
            &WatchOptions::default().metadata_only(),
        )
        .await
        .unwrap();
    let variants: Vec<FeedVariant> = ports
        .feeds()
        .stats()
        .per_feed
        .iter()
        .map(|f| f.variant)
        .collect();
    assert_eq!(variants, vec![FeedVariant::Full, FeedVariant::Metadata]);
}

#[tokio::test]
async fn a_watch_over_the_limit_is_refused_without_a_request() {
    let api = server();
    api.reply(PODS, 200, pod_list(vec![]));
    let config = BudgetConfig {
        max_feeds: 1,
        ..roomy()
    };
    let ports = budgeted(&api, config);
    let _held = ports
        .watch(&pods(), Some("default"), &WatchOptions::default())
        .await
        .unwrap();
    let hits = api.hits(PODS);
    let err = ports
        .table_feed(&pods(), Some("default"), &TableOptions::default())
        .await
        .err()
        .expect("refused");
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert_eq!(api.hits(PODS), hits, "nothing was sent to the server");
}

#[tokio::test]
async fn a_table_feed_is_a_table_feed_of_the_registry() {
    let api = server();
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_item("a", "1"), pod_item("b", "2")]),
    );
    let ports = budgeted(&api, roomy());
    let mut feed = ports
        .table_feed(&pods(), Some("default"), &TableOptions::default())
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(60), feed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(first.rows.contains_restart());
    let stats = ports.feeds().stats();
    assert_eq!(stats.per_feed[0].variant, FeedVariant::Table);
    assert_eq!(stats.objects, 2);
}

#[tokio::test]
async fn lists_and_gets_are_not_feeds() {
    let api = server();
    api.reply(PODS, 200, pod_list(vec![pod_item("a", "1")]));
    let ports = budgeted(&api, roomy());
    let page = ports
        .list(&pods(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let stats = ports.feeds().stats();
    assert_eq!((stats.started, stats.bytes), (0, 0));
}
