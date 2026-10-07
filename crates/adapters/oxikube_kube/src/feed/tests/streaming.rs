//! Streaming lists (`sendInitialEvents`): chosen from the server version, with a fallback.

use oxikube_domain::session::WatchScope;
use oxikube_ports::WatchOptions;

use super::server::*;
use super::*;

fn auto() -> FeedConfig {
    FeedConfig {
        streaming_lists: StreamingLists::Auto,
        ..config()
    }
}

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

#[tokio::test(start_paused = true)]
async fn a_1_32_server_gets_a_streaming_list() {
    let server = FeedServer::new(32);
    server.watch(
        PODS,
        Reply::Events(vec![
            event("ADDED", pod("a", "ua", "5")),
            event("ADDED", pod("b", "ub", "6")),
            initial_events_end("7"),
        ]),
    );
    let mut feed = open(&server, auto()).await;

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a,b]"]);
    assert_eq!(server.list_hits(PODS), 0, "no LIST request");
    let first = &server.queries(PODS)[0];
    assert!(first.contains("sendInitialEvents=true"), "{first}");
    assert!(
        first.contains("resourceVersionMatch=NotOlderThan"),
        "{first}"
    );
    assert_eq!(server.queries("/version").len(), 1);

    // The version is read once per cluster, not per feed.
    let resources = server.resources(auto());
    let _a = resources
        .reflector_feed(&pod_gvk(), &WatchScope::Cluster, &WatchOptions::default())
        .await
        .unwrap();
    let _b = resources
        .reflector_feed(&pod_gvk(), &WatchScope::Cluster, &WatchOptions::default())
        .await
        .unwrap();
    assert_eq!(
        server.queries("/version").len(),
        2,
        "one more read for the new adapter"
    );
}

#[tokio::test(start_paused = true)]
async fn an_older_server_gets_paged_lists() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    let mut feed = open(&server, auto()).await;

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a]"]);
    assert_eq!(server.list_hits(PODS), 1);
    assert!(
        server
            .queries(PODS)
            .iter()
            .all(|q| !q.contains("sendInitialEvents"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_rejected_streaming_list_falls_back_to_paged_lists() {
    let server = FeedServer::new(34);
    let invalid = crate::fake_api::status_body(422, "Invalid", "sendInitialEvents is forbidden");
    server.watch(PODS, Reply::Json(422, invalid));
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    let mut feed = open(&server, auto()).await;

    let first = next(&mut feed)
        .await
        .expect("no error: the fallback is silent");
    assert_eq!(labels(&first), vec!["restart[a]"]);
    assert_eq!(server.list_hits(PODS), 1);
    assert!(server.queries(PODS)[0].contains("sendInitialEvents=true"));
}

/// The streaming list is served from the API server's watch cache (kube sends
/// `resourceVersion=0`), which can trail etcd under load: a pod created just before the feed
/// opened may be missing from the opening list. It then arrives as a live event, so the
/// consumer still converges, with no relist.
#[tokio::test(start_paused = true)]
async fn a_lagging_opening_list_catches_up_through_live_events() {
    let server = FeedServer::new(34);
    server.watch(
        PODS,
        Reply::Events(vec![
            event("ADDED", pod("a", "ua", "5")),
            initial_events_end("6"),
            event("ADDED", pod("b", "ub", "7")),
            event("ADDED", pod("c", "uc", "8")),
        ]),
    );
    let mut feed = open(&server, auto()).await;

    let mut folded = Folded::default();
    let opening = next_batch(&mut feed).await;
    assert_eq!(labels(&opening), vec!["restart[a]"], "the lagging list");
    folded.apply(opening);
    while folded.0.len() < 3 {
        let batch = next_batch(&mut feed).await;
        assert!(
            batch.deltas.iter().all(|d| matches!(d, Delta::Applied(_))),
            "later batches are live changes, not another restart: {:?}",
            labels(&batch)
        );
        folded.apply(batch);
    }
    assert_eq!(folded, Folded::of(&[("a", "5"), ("b", "7"), ("c", "8")]));
    assert_eq!(server.list_hits(PODS), 0, "no LIST request");
}
