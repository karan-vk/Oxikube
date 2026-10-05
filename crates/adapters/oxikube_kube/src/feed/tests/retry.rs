//! Failures: backoff after a dropped connection and after 410 Gone, state transitions,
//! fatal errors, and abort on drop.

use oxikube_domain::ErrorKind;
use oxikube_domain::session::WatchScope;
use oxikube_ports::WatchOptions;

use super::server::*;
use super::*;
use crate::feed::{FeedState, RelistDelivery};
use crate::resources::is_list_expired;

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
async fn a_dropped_connection_backs_off_reports_retrying_and_resumes() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    server.watch(PODS, Reply::Drop);
    server.watch(
        PODS,
        Reply::Events(vec![event("MODIFIED", pod("a", "ua", "11"))]),
    );
    let mut feed = open(&server, config()).await;
    let state = feed.state();

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a]"]);
    let err = next(&mut feed).await.unwrap_err();
    assert!(err.is_retryable(), "{err}");
    assert_eq!(err.kind(), ErrorKind::Network);
    assert_eq!(*state.borrow(), FeedState::Retrying);

    let started = tokio::time::Instant::now();
    assert_eq!(labels(&next_batch(&mut feed).await), vec!["+a@11"]);
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "backed off first"
    );
    assert_eq!(*state.borrow(), FeedState::Live);
    assert_eq!(
        server.list_hits(PODS),
        1,
        "resumed from the resource version, no relist"
    );
    let watches = server.queries(PODS);
    assert!(watches[2].contains("resourceVersion=10"), "{}", watches[2]);
}

#[tokio::test(start_paused = true)]
async fn gone_relists_and_delivers_only_the_diff() {
    let server = FeedServer::new(31);
    let first = vec![
        pod("a", "ua", "1"),
        pod("b", "ub", "2"),
        pod("c", "uc", "3"),
    ];
    let second = vec![
        pod("a", "ua", "1"),
        pod("b", "ub", "20"),
        pod("d", "ud", "21"),
    ];
    server.list(PODS, pod_list(first, "10"));
    server.list(PODS, pod_list(second, "30"));
    server.watch(PODS, Reply::Events(vec![error_event(410, "Expired")]));
    server.watch(
        PODS,
        Reply::Events(vec![event("MODIFIED", pod("d", "ud", "31"))]),
    );
    let mut feed = open(&server, config()).await;
    let state = feed.state();

    let mut folded = Folded::default();
    let opening = next_batch(&mut feed).await;
    assert_eq!(labels(&opening), vec!["restart[a,b,c]"]);
    folded.apply(opening);

    let err = next(&mut feed).await.unwrap_err();
    assert!(is_list_expired(&err), "{err}");
    assert!(err.is_retryable());
    assert_eq!(*state.borrow(), FeedState::Retrying);

    let mut seen = Vec::new();
    while folded != Folded::of(&[("a", "1"), ("b", "20"), ("d", "31")]) {
        let batch = next_batch(&mut feed).await;
        assert!(!batch.contains_restart(), "a relist is sent as its diff");
        seen.extend(labels(&batch));
        folded.apply(batch);
    }
    // `+d@21` (relist) and `+d@31` (watch) fell in one window and merged.
    assert_eq!(seen, vec!["+b@20", "+d@31", "-c@3"]);
    assert_eq!(server.list_hits(PODS), 2);
    assert_eq!(*state.borrow(), FeedState::Live);
}

#[tokio::test(start_paused = true)]
async fn snapshot_delivery_sends_a_full_restart_after_a_relist() {
    let server = FeedServer::new(31);
    server.list(
        PODS,
        pod_list(vec![pod("a", "ua", "1"), pod("c", "uc", "3")], "10"),
    );
    server.list(
        PODS,
        pod_list(vec![pod("a", "ua", "1"), pod("d", "ud", "21")], "30"),
    );
    server.watch(PODS, Reply::Events(vec![error_event(410, "Expired")]));
    let config = FeedConfig {
        relist: RelistDelivery::Snapshot,
        ..config()
    };
    let mut feed = open(&server, config).await;

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a,c]"]);
    assert!(next(&mut feed).await.is_err());
    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a,d]"]);
}

#[tokio::test(start_paused = true)]
async fn a_forbidden_list_is_the_last_item() {
    let server = FeedServer::new(31);
    server.list(
        PODS,
        Reply::Json(403, crate::fake_api::status_body(403, "Forbidden", "no")),
    );
    let mut feed = open(&server, config()).await;
    let state = feed.state();

    let err = next(&mut feed).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(!err.is_retryable());
    let end = tokio::time::timeout(Duration::from_secs(60), futures::StreamExt::next(&mut feed));
    assert!(
        end.await.unwrap().is_none(),
        "the feed ends after a final error"
    );
    assert_eq!(*state.borrow(), FeedState::Stopped);
    assert!(feed.is_finished());
}

#[tokio::test(start_paused = true)]
async fn a_retry_settles_back_to_live_on_a_quiet_watch() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    server.watch(PODS, Reply::Drop);
    // The reconnect is accepted and only sends a bookmark: alive, nothing changed.
    server.watch(PODS, Reply::Events(vec![bookmark("10")]));
    let mut feed = open(&server, config()).await;
    let mut state = feed.state();

    next_batch(&mut feed).await;
    next(&mut feed).await.unwrap_err();
    assert_eq!(*state.borrow(), FeedState::Retrying);
    let started = tokio::time::Instant::now();
    let settled = tokio::time::timeout(
        Duration::from_secs(60),
        state.wait_for(|s| *s == FeedState::Live),
    );
    settled.await.unwrap().unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(10),
        "after retry_settle"
    );
}

/// Backoff delays longer than `retry_settle` (10 s).
fn slow_backoff() -> FeedConfig {
    FeedConfig {
        backoff_min: Duration::from_secs(20),
        backoff_max: Duration::from_secs(20),
        ..config()
    }
}

#[tokio::test(start_paused = true)]
async fn retrying_lasts_while_reconnects_keep_failing() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    // Refused connections, and answers that are not an accepted watch.
    let refused = || {
        Reply::Json(
            500,
            crate::fake_api::status_body(500, "InternalError", "down"),
        )
    };
    for reply in [Reply::Drop, refused(), Reply::Drop, refused()] {
        server.watch(PODS, reply);
    }
    server.watch(PODS, Reply::Events(vec![bookmark("10")]));
    let mut feed = open(&server, slow_backoff()).await;
    let mut state = feed.state();

    next_batch(&mut feed).await;
    next(&mut feed).await.unwrap_err();
    assert_eq!(*state.borrow(), FeedState::Retrying);
    let started = tokio::time::Instant::now();
    // Attempts at 20, 40 and 60 s fail; the one at 80 s is accepted and settles at 90 s.
    let settled = tokio::time::timeout(
        Duration::from_secs(600),
        state.wait_for(|s| *s != FeedState::Retrying),
    );
    assert_eq!(*settled.await.unwrap().unwrap(), FeedState::Live);
    assert!(
        started.elapsed() >= Duration::from_secs(90),
        "left Retrying after {:?}, before a reconnect got through",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn retrying_lasts_while_a_reconnect_hangs() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    server.watch(PODS, Reply::Drop);
    server.watch(PODS, Reply::Hang);
    let mut feed = open(&server, config()).await;
    let state = feed.state();

    next_batch(&mut feed).await;
    next(&mut feed).await.unwrap_err();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(server.open_watches(), 1, "the reconnect is still in flight");
    assert_eq!(*state.borrow(), FeedState::Retrying);
}

#[tokio::test(start_paused = true)]
async fn a_failing_initial_list_stays_retrying_until_objects_arrive() {
    let server = FeedServer::new(31);
    server.list(PODS, Reply::Drop);
    server.list(PODS, Reply::Hang);
    let mut feed = open(&server, config()).await;
    let state = feed.state();

    let err = next(&mut feed).await.unwrap_err();
    assert!(err.is_retryable(), "{err}");
    assert_eq!(*state.borrow(), FeedState::Retrying);
    // The watcher restarts its list (`Init`) after the backoff; that alone is no recovery.
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(server.list_hits(PODS), 2, "the list was retried");
    assert_eq!(*state.borrow(), FeedState::Retrying);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_feed_aborts_its_watches() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    let mut feed = open(&server, config()).await;
    let state = feed.state();
    next_batch(&mut feed).await;
    while server.open_watches() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(server.open_watches(), 1);

    drop(feed);
    for _ in 0..100 {
        if server.open_watches() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        server.open_watches(),
        0,
        "the hanging watch request was cancelled"
    );
    assert_eq!(*state.borrow(), FeedState::Stopped);
}
