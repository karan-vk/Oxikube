//! One API per cluster: discovery picks `events.k8s.io/v1` or falls back to `core/v1`, a
//! forbidden API falls back to the other, a forbidden cluster-wide watch falls back to the
//! namespaces the caller names, and the forbidden scopes are reported.

use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::ErrorKind;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, EventApiChoice, EventsPort, ForbiddenScope};

use super::feed::*;
use super::*;
use crate::events::{EventApi, EventFeed, EventsConfig, EventsOptions};
use crate::fake_api::FakeApi;
use crate::feed::FeedState;

const V1_A: &str = "/apis/events.k8s.io/v1/namespaces/a/events";
const V1_B: &str = "/apis/events.k8s.io/v1/namespaces/b/events";
const CORE_A: &str = "/api/v1/namespaces/a/events";
const CORE_B: &str = "/api/v1/namespaces/b/events";

async fn auto(api: &FakeApi, options: &EventsOptions) -> EventFeed {
    open_with(api, &cluster_scope(), options, EventsConfig::default()).await
}

#[tokio::test(start_paused = true)]
async fn auto_watches_exactly_one_api_and_prefers_events_k8s_io() {
    let api = server(true);
    api.reply(V1, 200, list(vec![v1_event("x", "web-0", T1, 3)]));
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 3)]));
    let mut feed = auto(&api, &EventsOptions::default()).await;
    assert_eq!(feed.apis(), [EventApi::EventsV1]);
    let first = next_batch(&mut feed).await;
    assert_eq!(restart_uids(&first), ["x"]);
    assert_eq!(*feed.state().borrow(), FeedState::Live);
    assert_eq!(api.hits(CORE), 0, "the second API is never requested");
    assert_eq!(feed.status().apis, [EventApi::EventsV1]);
    assert!(feed.forbidden().is_empty());
}

#[tokio::test(start_paused = true)]
async fn auto_falls_back_to_core_when_discovery_does_not_serve_events_k8s_io() {
    let api = server(false);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let mut feed = auto(&api, &EventsOptions::default()).await;
    assert_eq!(feed.apis(), [EventApi::Core]);
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(
        api.hits(V1),
        0,
        "detected by discovery, not by a failed watch"
    );
}

#[tokio::test(start_paused = true)]
async fn core_only_forces_the_fallback_even_when_events_k8s_io_is_served() {
    let api = server(true);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let options = EventsOptions::default().api(EventApiChoice::CoreOnly);
    let mut feed = auto(&api, &options).await;
    assert_eq!(feed.apis(), [EventApi::Core]);
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(api.hits(V1), 0);
}

#[tokio::test(start_paused = true)]
async fn events_k8s_io_forbidden_falls_back_to_core_without_an_error() {
    let api = server(true);
    api.reply(V1, 403, forbidden());
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let mut feed = auto(&api, &EventsOptions::default()).await;
    // The first item is the opening list, not the refusal.
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(feed.apis(), [EventApi::Core]);
    assert!(
        feed.forbidden().is_empty(),
        "core is readable, nothing is hidden"
    );
    assert_eq!(*feed.state().borrow(), FeedState::Live);
    assert_eq!(api.hits(V1), 1, "the refused watch is not retried");
}

#[tokio::test(start_paused = true)]
async fn when_both_apis_are_forbidden_the_cluster_is_reported_forbidden() {
    let api = server(true);
    api.reply(V1, 403, forbidden());
    api.reply(CORE, 403, forbidden());
    let mut feed = auto(&api, &EventsOptions::default()).await;
    let err = next(&mut feed).await.expect_err("the final error");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(feed.next().await.is_none());
    assert_eq!(feed.forbidden(), [ForbiddenScope::ClusterWide]);
}

#[tokio::test(start_paused = true)]
async fn a_forbidden_cluster_wide_watch_falls_back_to_the_named_namespaces() {
    let api = server(true);
    api.reply(V1, 403, forbidden());
    api.reply(CORE, 403, forbidden());
    api.reply(V1_A, 200, list(vec![v1_event("a1", "web-0", T1, 1)]));
    api.reply(V1_B, 403, forbidden());
    api.reply(CORE_B, 403, forbidden());
    let options = EventsOptions::default().fallback_namespaces(["b".to_owned(), "a".to_owned()]);
    let mut feed = auto(&api, &options).await;
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["a1"]);
    settle().await;
    assert_eq!(feed.apis(), [EventApi::EventsV1]);
    assert_eq!(
        feed.forbidden(),
        [ForbiddenScope::Namespace("b".into())],
        "the namespace the user may not read is reported, the cluster-wide refusal is not"
    );
    assert_eq!(api.hits(CORE_A), 0, "one API per namespace");
    assert!(
        api.hits(CORE_B) >= 1,
        "b is tried on core before it is called forbidden on both APIs"
    );
}

#[tokio::test(start_paused = true)]
async fn a_namespace_refused_on_events_k8s_io_is_read_on_core_next_to_one_that_works() {
    let api = server(true);
    api.reply(V1, 403, forbidden());
    api.reply(CORE, 403, forbidden());
    api.reply(V1_A, 200, list(vec![v1_event("a1", "web-0", T1, 1)]));
    api.reply(V1_B, 403, forbidden());
    api.reply(CORE_B, 200, list(vec![core_event("b1", "web-1", T2, 1)]));
    let options = EventsOptions::default().fallback_namespaces(["a".to_owned(), "b".to_owned()]);
    let mut feed = auto(&api, &options).await;
    let mut uids = restart_uids(&next_batch(&mut feed).await);
    uids.sort();
    assert_eq!(uids, ["a1", "b1"], "the opening list holds both namespaces");
    settle().await;
    assert_eq!(feed.apis(), [EventApi::EventsV1, EventApi::Core]);
    assert!(
        feed.forbidden().is_empty(),
        "b is readable on core, nothing is hidden"
    );
    assert_eq!(api.hits(V1_B), 1, "the refused watch is not retried");
    assert_eq!(api.hits(CORE_A), 0, "a stays on its one API");
    assert_eq!(*feed.state().borrow(), FeedState::Live);
}

#[tokio::test(start_paused = true)]
async fn the_namespace_fallback_tries_core_when_events_k8s_io_is_forbidden_there_too() {
    let api = server(true);
    api.reply(V1, 403, forbidden());
    api.reply(CORE, 403, forbidden());
    api.reply(V1_A, 403, forbidden());
    api.reply(CORE_A, 200, list(vec![core_event("a1", "web-0", T1, 1)]));
    let options = EventsOptions::default().fallback_namespaces(["a".to_owned()]);
    let mut feed = auto(&api, &options).await;
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["a1"]);
    assert_eq!(feed.apis(), [EventApi::Core]);
    assert!(feed.forbidden().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_namespace_scope_reports_the_forbidden_namespaces_and_keeps_the_rest() {
    let api = server(false);
    api.reply(CORE_A, 200, list(vec![core_event("a1", "web-0", T1, 1)]));
    api.reply(CORE_B, 403, forbidden());
    let scope = WatchScope::Namespaces(vec!["a".into(), "b".into()]);
    let mut feed = open_with(
        &api,
        &scope,
        &EventsOptions::default(),
        EventsConfig::default(),
    )
    .await;
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["a1"]);
    settle().await;
    assert_eq!(feed.forbidden(), [ForbiddenScope::Namespace("b".into())]);
    assert!(!feed.is_finished());
}

#[tokio::test(start_paused = true)]
async fn the_port_hands_out_the_feed_and_a_monitor_that_outlives_polling() {
    let api = server(false);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let port = events(&api, EventsConfig::default());
    let port: &dyn EventsPort = &port;
    let mut served = port
        .watch_events(&WatchScope::Cluster, &EventsOptions::default())
        .await
        .unwrap();
    let batch = tokio::time::timeout(Duration::from_secs(600), served.feed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(restart_uids(&batch), ["x"]);
    let status = served.monitor.status();
    assert_eq!((status.len, status.apis), (1, vec![EventApi::Core]));
    assert!(status.capacity > 0);
}

#[tokio::test(start_paused = true)]
async fn a_cluster_wide_fallback_without_namespaces_is_the_final_error() {
    let api = server(false);
    api.reply(CORE, 403, forbidden());
    let mut feed = auto(&api, &EventsOptions::default()).await;
    assert_eq!(
        next(&mut feed).await.unwrap_err().kind(),
        ErrorKind::Forbidden
    );
    assert_eq!(feed.forbidden(), [ForbiddenScope::ClusterWide]);
}

#[tokio::test(start_paused = true)]
async fn an_empty_fallback_namespace_is_a_validation_error() {
    let api = server(false);
    let err = events(&api, EventsConfig::default())
        .watch(
            &WatchScope::Cluster,
            &EventsOptions::default().fallback_namespaces([String::new()]),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

fn fixture(path: &str) -> serde_json::Value {
    serde_json::from_str(oxikube_testkit::fixtures::raw(path).expect("a testkit fixture"))
        .expect("fixture JSON")
}

/// Both realistic fixtures (the testkit's `events/` manifests) go through the real watcher
/// on the API the cluster serves, and come out as the same normalised shape.
#[tokio::test(start_paused = true)]
async fn the_testkit_fixtures_of_both_apis_normalise_to_one_event() {
    // A cluster serving events.k8s.io/v1: the events.k8s.io fixture, with its series.
    let api = server(true);
    api.reply(V1, 200, list(vec![fixture("events/events-v1.json")]));
    let mut feed = auto(&api, &EventsOptions::default()).await;
    let Delta::Restarted(all) = &next_batch(&mut feed).await.deltas[0] else {
        unreachable!("the opening batch")
    };
    let v1 = &all[0];
    assert_eq!((v1.count, &*v1.reason), (2, "ScalingReplicaSet"));
    assert_eq!(v1.regarding.gvk.kind.as_ref(), "Deployment");
    assert_eq!(v1.last_seen, Some("2026-01-01T00:10:00Z".parse().unwrap()));
    assert_eq!(v1.namespace.as_deref(), Some("demo"));

    // A cluster without it: the core/v1 fixture, with its legacy count and timestamps.
    let api = server(false);
    api.reply(CORE, 200, list(vec![fixture("events/core-warning.json")]));
    let mut feed = auto(&api, &EventsOptions::default()).await;
    let Delta::Restarted(all) = &next_batch(&mut feed).await.deltas[0] else {
        unreachable!("the opening batch")
    };
    let core = &all[0];
    assert_eq!((core.count, &*core.reason), (42, "BackOff"));
    assert!(core.is_warning());
    assert_eq!(core.regarding.gvk.kind.as_ref(), "Pod");
    assert_eq!(
        core.last_seen,
        Some("2026-01-01T01:00:00Z".parse().unwrap())
    );
    assert_eq!(core.reporting_component.as_deref(), Some("kubelet"));
}
