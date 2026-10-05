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
