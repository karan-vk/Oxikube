//! What a consumer receives: the opening `Restarted`, ordered deltas, coalescing,
//! backpressure, selectors and scopes.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{ResourceReader, WatchOptions};

use super::server::*;
use super::*;
use crate::feed::FeedState;

fn default_ns() -> WatchScope {
    WatchScope::Namespaces(vec!["default".into()])
}

#[tokio::test(start_paused = true)]
async fn the_first_batch_is_one_restart_then_applied_and_deleted() {
    let server = FeedServer::new(31);
    server.list(
        PODS,
        pod_list(vec![pod("a", "ua", "1"), pod("b", "ub", "2")], "10"),
    );
    server.watch(
        PODS,
        Reply::Events(vec![
            event("MODIFIED", pod("a", "ua", "11")),
            event("ADDED", pod("c", "uc", "12")),
            event("DELETED", pod("b", "ub", "13")),
        ]),
    );
    let mut feed = server
        .resources(config())
        .reflector_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();
    assert_eq!(feed.key().gvk, pod_gvk());

    let first = next_batch(&mut feed).await;
    assert_eq!(labels(&first), vec!["restart[a,b]"]);
    let second = next_batch(&mut feed).await;
    assert_eq!(labels(&second), vec!["+a@11", "+c@12", "-b@13"]);
    assert_eq!(second.resource_version.as_deref(), Some("13"));

    assert_eq!(*feed.state().borrow(), FeedState::Live);
    let mut stored: Vec<_> = feed
        .snapshot()
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    stored.sort();
    assert_eq!(stored, vec!["a", "c"]);
    let a = feed
        .snapshot()
        .into_iter()
        .find(|o| o.name() == "a")
        .unwrap();
    assert!(
        a.get("/metadata/managedFields").is_none(),
        "managedFields stripped"
    );
    assert_eq!(a.kind, pod_gvk());
}

#[tokio::test(start_paused = true)]
async fn a_thousand_changes_in_one_window_make_one_merged_batch() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![], "1"));
    // 100 pods, each added and then modified nine times: 1 000 events in one burst.
    let mut events = Vec::new();
    for round in 0..10 {
        for p in 0..100 {
            let kind = if round == 0 { "ADDED" } else { "MODIFIED" };
            let rv = (2 + round * 100 + p).to_string();
            events.push(event(kind, pod(&format!("p{p}"), &format!("u{p}"), &rv)));
        }
    }
    server.watch(PODS, Reply::Events(events));
    let mut feed = server
        .resources(config())
        .reflector_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();

    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    let batch = next_batch(&mut feed).await;
    assert_eq!(batch.len(), 100, "one delta per pod, the latest");
    folded.apply(batch);
    assert_eq!(folded.0.len(), 100);
    assert_eq!(folded.0["p0"], "902");
    assert_eq!(folded.0["p99"], "1001");
}

#[tokio::test(start_paused = true)]
async fn max_batch_splits_a_burst_of_distinct_objects() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![], "1"));
    let events = (0..1000)
        .map(|p| {
            event(
                "ADDED",
                pod(&format!("p{p}"), &format!("u{p}"), &(p + 2).to_string()),
            )
        })
        .collect();
    server.watch(PODS, Reply::Events(events));
    let config = FeedConfig {
        max_batch: 256,
        ..config()
    };
    let mut feed = server
        .resources(config)
        .reflector_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();

    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    let mut batches = 0;
    while folded.0.len() < 1000 {
        let batch = next_batch(&mut feed).await;
        assert!(batch.len() <= 256, "a batch never exceeds max_batch");
        folded.apply(batch);
        batches += 1;
    }
    assert!(batches <= 5, "1 000 events made {batches} batches");
}

#[tokio::test(start_paused = true)]
async fn a_slow_consumer_gets_merged_batches_not_a_queue() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("p0", "u0", "1")], "1"));
    let mut events = Vec::new();
    for round in 0..50 {
        for p in 0..10 {
            let rv = (2 + round * 10 + p).to_string();
            events.push(event(
                "MODIFIED",
                pod(&format!("p{p}"), &format!("u{p}"), &rv),
            ));
        }
    }
    server.watch(PODS, Reply::Events(events));
    let config = FeedConfig {
        channel_capacity: 1,
        ..config()
    };
    let mut feed = server
        .resources(config)
        .reflector_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();
    // Do not read while the 500 events arrive: the opening batch fills the channel.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let mut folded = Folded::default();
    let first = next_batch(&mut feed).await;
    assert!(first.contains_restart());
    folded.apply(first);
    let merged = next_batch(&mut feed).await;
    assert_eq!(merged.len(), 10, "500 queued events merged to one per pod");
    folded.apply(merged);
    assert_eq!(folded.0.len(), 10);
    assert_eq!(folded.0["p9"], "501");
}

#[tokio::test(start_paused = true)]
async fn selectors_page_size_and_timeout_reach_the_server() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![], "1"));
    let options = WatchOptions::default()
        .labels("app=web")
        .fields("spec.nodeName=n1")
        .page_size(7);
    let mut feed = server
        .resources(config())
        .reflector_feed(&pod_gvk(), &default_ns(), &options)
        .await
        .unwrap();
    next_batch(&mut feed).await;
    while server.queries(PODS).len() < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let decoded = |raw: &str| {
        url::form_urlencoded::parse(raw.as_bytes())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
    };
    let queries = server.queries(PODS);
    let list = decoded(&queries[0]);
    for expected in [
        "labelSelector=app=web",
        "fieldSelector=spec.nodeName=n1",
        "limit=7",
    ] {
        assert!(list.contains(&expected.to_owned()), "list {list:?}");
    }
    let watch = decoded(&queries[1]);
    for expected in [
        "watch=true",
        "labelSelector=app=web",
        "fieldSelector=spec.nodeName=n1",
        "timeoutSeconds=290",
        "allowWatchBookmarks=true",
        "resourceVersion=1",
    ] {
        assert!(watch.contains(&expected.to_owned()), "watch {watch:?}");
    }
    assert_eq!(
        server.watch_encodings(),
        vec![Some("identity".to_owned())],
        "watches ask for an uncompressed body (feed::transport)"
    );
}

#[tokio::test(start_paused = true)]
async fn a_namespace_set_merges_one_watch_per_namespace() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "5"));
    server.list(
        OTHER_PODS,
        pod_list(vec![pod_in("other", "x", "ux", "2")], "5"),
    );
    server.watch(
        OTHER_PODS,
        Reply::Events(vec![event("ADDED", pod_in("other", "y", "uy", "6"))]),
    );
    let scope = WatchScope::Namespaces(vec!["default".into(), "other".into()]);
    let mut feed = server
        .resources(config())
        .reflector_feed(&pod_gvk(), &scope, &WatchOptions::default())
        .await
        .unwrap();

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a,x]"]);
    assert_eq!(labels(&next_batch(&mut feed).await), vec!["+y@6"]);
    assert_eq!(feed.snapshot().len(), 3);
    assert_eq!(server.list_hits(ALL_PODS), 0, "no cluster-wide request");
}

#[tokio::test(start_paused = true)]
async fn the_port_watch_opens_the_same_feed_and_validates_its_target() {
    let server = FeedServer::new(31);
    server.list(ALL_PODS, pod_list(vec![pod("a", "ua", "1")], "5"));
    let resources = server.resources(config());
    let options = WatchOptions::default();

    let mut feed = resources.watch(&pod_gvk(), None, &options).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(60), futures::StreamExt::next(&mut feed))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(first.contains_restart());

    let namespace = Gvk::new("", "v1", "Namespace");
    let err = resources
        .watch(&namespace, Some("default"), &options)
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), ErrorKind::Validation);
    let binding = Gvk::new("", "v1", "Binding");
    let err = resources
        .watch(&binding, None, &options)
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let empty = WatchScope::Namespaces(vec![]);
    let err = resources
        .reflector_feed(&pod_gvk(), &empty, &options)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}
