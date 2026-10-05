//! The feed end to end against a scripted API server: both APIs merged, per-object filters,
//! fallbacks, the ring through the stream, failures and drop.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiResult;
use oxikube_domain::event::Event;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use serde_json::{Value, json};

use super::*;
use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::events::{EventApi, EventApis, EventFeed, EventsConfig, EventsOptions, KubeEvents};
use crate::fake_api::{FakeApi, status_body};
use crate::feed::{FeedConfig, FeedState, StreamingLists};
use crate::resources::KubeResources;

const CORE: &str = "/api/v1/events";
const V1: &str = "/apis/events.k8s.io/v1/events";
const CORE_DEFAULT: &str = "/api/v1/namespaces/default/events";
const CORE_OTHER: &str = "/api/v1/namespaces/other/events";
const T1: &str = "2026-10-03T11:01:00Z";
const T2: &str = "2026-10-03T11:02:00Z";
const T3: &str = "2026-10-03T11:03:00Z";
const T4: &str = "2026-10-03T11:04:00Z";
const T5: &str = "2026-10-03T11:05:00Z";

fn resource(name: &str, kind: &str) -> Value {
    json!({"name": name, "singularName": "", "namespaced": true, "kind": kind,
        "verbs": ["get", "list", "watch"]})
}

/// A server whose discovery serves `Event` on the core group and, with `with_v1`, on
/// `events.k8s.io/v1`.
fn server(with_v1: bool) -> FakeApi {
    let api = FakeApi::new();
    let gv = json!({"groupVersion": "events.k8s.io/v1", "version": "v1"});
    let groups = if with_v1 {
        json!([{"name": "events.k8s.io", "versions": [gv], "preferredVersion": gv}])
    } else {
        json!([])
    };
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply(
        "/apis",
        200,
        json!({"kind": "APIGroupList", "groups": groups}),
    );
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1",
            "resources": [resource("events", "Event")]}),
    );
    if with_v1 {
        api.reply(
            "/apis/events.k8s.io/v1",
            200,
            json!({"kind": "APIResourceList", "groupVersion": "events.k8s.io/v1",
                "resources": [resource("events", "Event")]}),
        );
    }
    api
}

fn feed_config() -> FeedConfig {
    FeedConfig {
        streaming_lists: StreamingLists::Never,
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_secs(1),
        backoff_jitter: false,
        ..FeedConfig::default()
    }
}

fn events(api: &FakeApi, config: EventsConfig) -> KubeEvents {
    let discovery = KubeDiscovery::with_config(
        api.client(),
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );
    let resources = KubeResources::new(api.client(), discovery).with_feed_config(feed_config());
    KubeEvents::new(resources, cluster()).with_config(config)
}

fn list(items: Vec<Value>) -> Value {
    json!({"kind": "EventList", "apiVersion": "v1",
        "metadata": {"resourceVersion": "10"}, "items": items})
}

fn watch_event(kind: &str, object: Value) -> Value {
    json!({"type": kind, "object": object})
}

fn forbidden() -> Value {
    status_body(403, "Forbidden", "events is forbidden")
}

async fn open(api: &FakeApi, scope: &WatchScope, options: &EventsOptions) -> EventFeed {
    events(api, EventsConfig::default())
        .watch(scope, options)
        .await
        .expect("the feed opens")
}

async fn next(feed: &mut EventFeed) -> OxiResult<DeltaBatch<Event>> {
    tokio::time::timeout(Duration::from_secs(600), feed.next())
        .await
        .expect("a feed item in time")
        .expect("the feed is still open")
}

async fn next_batch(feed: &mut EventFeed) -> DeltaBatch<Event> {
    next(feed)
        .await
        .unwrap_or_else(|e| panic!("expected a batch, got {e}"))
}

/// The events of the opening `Restarted`, by uid.
fn restart_uids(batch: &DeltaBatch<Event>) -> Vec<String> {
    let [Delta::Restarted(all)] = batch.deltas.as_slice() else {
        panic!("expected one Restarted, got {:?}", batch.deltas);
    };
    all.iter()
        .map(|e| e.uid.as_deref().unwrap().to_owned())
        .collect()
}

/// What a consumer folding the deltas holds: uid to count.
#[derive(Default, Debug, PartialEq)]
struct Folded(BTreeMap<String, u32>);

impl Folded {
    fn apply(&mut self, batch: DeltaBatch<Event>) {
        for delta in batch {
            match delta {
                Delta::Restarted(all) => {
                    self.0 = all.iter().map(|e| (uid(e), e.count)).collect();
                }
                Delta::Applied(e) => {
                    self.0.insert(uid(&e), e.count);
                }
                Delta::Deleted(e) => {
                    self.0.remove(&uid(&e));
                }
            }
        }
    }
}

fn uid(e: &Event) -> String {
    e.uid.as_deref().unwrap().to_owned()
}

/// Lets the paused clock run the feed's tasks until they are idle.
async fn settle() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

fn cluster_scope() -> WatchScope {
    WatchScope::Cluster
}

#[tokio::test(start_paused = true)]
async fn both_apis_are_merged_into_one_event_each() {
    let api = server(true);
    // x and y exist on both APIs, as the server does; only core has z, only v1 has w.
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("x", "web-0", T3, 7),
            core_event("y", "web-1", T2, 1),
            core_event("z", "web-2", T1, 1),
        ]),
    );
    api.reply(
        V1,
        200,
        list(vec![
            v1_event("x", "web-0", T3, 7),
            v1_event("y", "web-1", T2, 1),
            v1_event("w", "web-3", T4, 2),
        ]),
    );
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    assert_eq!(feed.apis(), [EventApi::Core, EventApi::EventsV1]);

    let first = next_batch(&mut feed).await;
    // Oldest last-seen first.
    assert_eq!(restart_uids(&first), ["z", "y", "x", "w"]);
    let Delta::Restarted(all) = &first.deltas[0] else {
        unreachable!()
    };
    let x = all.iter().find(|e| uid(e) == "x").unwrap();
    assert_eq!((x.count, &*x.reason), (7, "BackOff"));
    assert_eq!(x.regarding_uid.as_deref(), Some("uid-of-web-0"));
    assert!(x.is_warning());
    let w = all.iter().find(|e| uid(e) == "w").unwrap();
    assert_eq!(
        w.message, "Back-off restarting failed container",
        "`note` is the message"
    );
    assert_eq!(feed.stats().len, 4);
    assert_eq!(feed.stats().evicted, 0);
    assert_eq!(*feed.state().borrow(), FeedState::Live);
}

#[tokio::test(start_paused = true)]
async fn objects_of_a_real_list_without_type_fields_are_mapped_per_api() {
    let api = server(true);
    let untyped = |mut v: Value| {
        v.as_object_mut().unwrap().remove("apiVersion");
        v.as_object_mut().unwrap().remove("kind");
        v
    };
    api.reply(
        CORE,
        200,
        list(vec![untyped(core_event("x", "web-0", T1, 1))]),
    );
    api.reply(V1, 200, list(vec![untyped(v1_event("w", "web-1", T2, 1))]));
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x", "w"]);
}

#[tokio::test(start_paused = true)]
async fn a_change_seen_on_both_apis_is_one_delta_and_a_delete_is_one_delete() {
    let api = server(true);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    api.reply(V1, 200, list(vec![v1_event("x", "web-0", T1, 1)]));
    // x recurs (count 2) and a new event n appears, on both APIs; then x is deleted on both.
    api.reply_watch(
        CORE,
        200,
        &[
            watch_event("MODIFIED", core_event("x", "web-0", T2, 2)),
            watch_event("ADDED", core_event("n", "web-0", T3, 1)),
            watch_event("DELETED", core_event("x", "web-0", T2, 2)),
        ],
    );
    api.reply_watch(
        V1,
        200,
        &[
            watch_event("MODIFIED", v1_event("x", "web-0", T2, 2)),
            watch_event("ADDED", v1_event("n", "web-0", T3, 1)),
            watch_event("DELETED", v1_event("x", "web-0", T2, 2)),
        ],
    );
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    let mut folded = Folded::default();
    let mut deltas = Vec::new();
    folded.apply(next_batch(&mut feed).await);
    while folded.0.contains_key("x") || !folded.0.contains_key("n") {
        let batch = next_batch(&mut feed).await;
        deltas.extend(batch.deltas.iter().cloned());
        folded.apply(batch);
    }
    settle().await;
    while let Ok(Some(Ok(batch))) = tokio::time::timeout(Duration::from_secs(5), feed.next()).await
    {
        deltas.extend(batch.deltas.iter().cloned());
        folded.apply(batch);
    }
    assert_eq!(folded.0, BTreeMap::from([("n".to_owned(), 1)]));
    let applied_n = deltas
        .iter()
        .filter(|d| matches!(d, Delta::Applied(e) if uid(e) == "n"))
        .count();
    assert_eq!(
        applied_n, 1,
        "n arrived on both APIs and was delivered once"
    );
}

#[tokio::test(start_paused = true)]
async fn per_object_feed_uses_each_apis_own_field_selector() {
    let api = server(true);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    api.reply(V1, 200, list(vec![v1_event("x", "web-0", T1, 1)]));
    let mut feed = open(
        &api,
        &cluster_scope(),
        &EventsOptions::for_object("uid-of-web-0"),
    )
    .await;
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);

    let query = |path: &str| {
        let raw = api
            .requests()
            .into_iter()
            .find(|r| r.path == path && !r.is_watch())
            .unwrap()
            .query;
        url::form_urlencoded::parse(raw.as_bytes())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
    };
    assert!(query(CORE).contains(&"fieldSelector=involvedObject.uid=uid-of-web-0".to_owned()));
    assert!(query(V1).contains(&"fieldSelector=regarding.uid=uid-of-web-0".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn the_client_filters_by_uid_when_the_server_does_not() {
    let api = server(true);
    // A server that ignores the selector returns everything, in the list and on the watch.
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("mine", "web-0", T1, 1),
            core_event("other", "web-1", T2, 1),
        ]),
    );
    api.reply(V1, 200, list(vec![v1_event("other", "web-1", T2, 1)]));
    api.reply_watch(
        CORE,
        200,
        &[
            watch_event("ADDED", core_event("other-2", "web-1", T3, 1)),
            watch_event("ADDED", core_event("mine-2", "web-0", T4, 1)),
            watch_event("DELETED", core_event("other", "web-1", T2, 1)),
        ],
    );
    let mut feed = open(
        &api,
        &cluster_scope(),
        &EventsOptions::for_object("uid-of-web-0"),
    )
    .await;
    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["mine"]);
    folded.apply(next_batch(&mut feed).await);
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["mine", "mine-2"]);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_field_selector_is_dropped_and_the_list_retried() {
    let api = server(false);
    api.reply(
        CORE,
        400,
        status_body(400, "BadRequest", "field label not supported"),
    );
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("mine", "web-0", T1, 1),
            core_event("other", "web-1", T2, 1),
        ]),
    );
    let mut feed = events(
        &api,
        EventsConfig {
            apis: EventApis::CoreOnly,
            ..EventsConfig::default()
        },
    )
    .watch(&cluster_scope(), &EventsOptions::for_object("uid-of-web-0"))
    .await
    .unwrap();
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["mine"]);
    let lists: Vec<_> = api
        .requests()
        .into_iter()
        .filter(|r| r.path == CORE && !r.is_watch())
        .collect();
    assert!(lists[0].query.contains("fieldSelector"));
    assert!(
        !lists[1].query.contains("fieldSelector"),
        "retried without the selector"
    );
}

#[tokio::test(start_paused = true)]
async fn a_cluster_without_events_k8s_io_uses_core_only() {
    let api = server(false);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    assert_eq!(feed.apis(), [EventApi::Core]);
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(api.hits(V1), 0, "an unserved API is not requested");
}

#[tokio::test(start_paused = true)]
async fn a_cluster_serving_neither_api_is_unsupported() {
    let api = FakeApi::new();
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply("/apis", 200, json!({"kind": "APIGroupList", "groups": []}));
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": []}),
    );
    let err = events(&api, EventsConfig::default())
        .watch(&cluster_scope(), &EventsOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test(start_paused = true)]
async fn a_forbidden_api_drops_out_while_the_other_carries_on() {
    let api = server(true);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    api.reply(V1, 403, forbidden());
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    // The first item is the opening list from core, not an error.
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(*feed.state().borrow(), FeedState::Live);
    assert!(!feed.is_finished());
}

#[tokio::test(start_paused = true)]
async fn when_every_api_is_forbidden_the_error_is_the_last_item() {
    let api = server(true);
    api.reply(CORE, 403, forbidden());
    api.reply(V1, 403, forbidden());
    let mut feed = open(&api, &cluster_scope(), &EventsOptions::default()).await;
    let err = next(&mut feed)
        .await
        .expect_err("an error, not an opening list");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(
        feed.next().await.is_none(),
        "the feed ends after its final error"
    );
    assert!(feed.is_finished());
}

#[tokio::test(start_paused = true)]
async fn the_ring_bounds_the_feed_and_reports_evictions() {
    let api = server(false);
    let times = [T1, T2, T3, T4, T5];
    api.reply(
        CORE,
        200,
        list(
            times
                .iter()
                .enumerate()
                .map(|(i, t)| core_event(&format!("e{i}"), "web-0", t, 1))
                .collect(),
        ),
    );
    api.reply_watch(
        CORE,
        200,
        &[
            watch_event(
                "ADDED",
                core_event("e5", "web-0", "2026-10-03T11:06:00Z", 1),
            ),
            watch_event(
                "ADDED",
                core_event("e6", "web-0", "2026-10-03T11:07:00Z", 1),
            ),
            // Older than everything held: not kept.
            watch_event(
                "ADDED",
                core_event("ancient", "web-0", "2026-10-03T09:00:00Z", 1),
            ),
        ],
    );
    let config = EventsConfig {
        capacity: 3,
        apis: EventApis::CoreOnly,
    };
    let mut feed = events(&api, config)
        .watch(&cluster_scope(), &EventsOptions::default())
        .await
        .unwrap();
    assert_eq!(feed.stats().capacity, 3);

    let first = next_batch(&mut feed).await;
    assert_eq!(
        restart_uids(&first),
        ["e2", "e3", "e4"],
        "the newest three of five"
    );

    let mut folded = Folded::default();
    folded.apply(first);
    let mut deltas = Vec::new();
    while !folded.0.contains_key("e6") {
        let batch = next_batch(&mut feed).await;
        deltas.extend(batch.deltas.iter().cloned());
        folded.apply(batch);
        assert!(
            folded.0.len() <= 3,
            "a consumer never holds more than the capacity"
        );
    }
    settle().await;
    let stats = feed.stats();
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["e4", "e5", "e6"]);
    assert_eq!(
        (stats.len, stats.evicted),
        (3, 5),
        "e2, e3 and the ancient one counted too"
    );
    // The eviction of e2 is sent before e5 arrives, as a Deleted.
    let first_delete = deltas
        .iter()
        .position(|d| matches!(d, Delta::Deleted(e) if uid(e) == "e2"));
    let e5 = deltas
        .iter()
        .position(|d| matches!(d, Delta::Applied(e) if uid(e) == "e5"));
    assert!(first_delete < e5);
}

#[tokio::test(start_paused = true)]
async fn a_relist_after_410_retires_what_vanished_and_keeps_the_rest() {
    let api = server(false);
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("keep", "web-0", T1, 1),
            core_event("gone", "web-0", T2, 1),
        ]),
    );
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("keep", "web-0", T1, 1),
            core_event("fresh", "web-0", T3, 1),
        ]),
    );
    api.reply_watch(
        CORE,
        200,
        &[json!({"type": "ERROR", "object": status_body(410, "Expired", "too old")})],
    );
    let mut feed = events(
        &api,
        EventsConfig {
            apis: EventApis::CoreOnly,
            ..EventsConfig::default()
        },
    )
    .watch(&cluster_scope(), &EventsOptions::default())
    .await
    .unwrap();
    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["gone", "keep"]);
    while folded.0.contains_key("gone") || !folded.0.contains_key("fresh") {
        match next(&mut feed).await {
            Ok(batch) => folded.apply(batch),
            Err(err) => assert!(err.is_retryable(), "the 410 is retried: {err}"),
        }
    }
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["fresh", "keep"]);
}

#[tokio::test(start_paused = true)]
async fn namespace_scopes_watch_each_namespace_and_relist_only_their_own() {
    let api = server(false);
    api.reply(
        CORE_DEFAULT,
        200,
        list(vec![
            core_event("d1", "web-0", T1, 1),
            core_event("d2", "web-0", T2, 1),
        ]),
    );
    api.reply(
        CORE_DEFAULT,
        200,
        list(vec![core_event("d1", "web-0", T1, 1)]),
    );
    api.reply(CORE_OTHER, 200, list(vec![core_event("o1", "db-0", T3, 1)]));
    api.reply_watch(
        CORE_DEFAULT,
        200,
        &[json!({"type": "ERROR", "object": status_body(410, "Expired", "too old")})],
    );
    let scope = WatchScope::Namespaces(vec!["default".into(), "other".into()]);
    let mut feed = events(
        &api,
        EventsConfig {
            apis: EventApis::CoreOnly,
            ..EventsConfig::default()
        },
    )
    .watch(&scope, &EventsOptions::default())
    .await
    .unwrap();
    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    assert_eq!(folded.0.keys().collect::<Vec<_>>(), ["d1", "d2", "o1"]);
    while folded.0.contains_key("d2") {
        if let Ok(batch) = next(&mut feed).await {
            folded.apply(batch);
        }
    }
    assert_eq!(
        folded.0.keys().collect::<Vec<_>>(),
        ["d1", "o1"],
        "other's event survives default's relist"
    );
}

#[tokio::test(start_paused = true)]
async fn objects_that_do_not_map_are_skipped_and_counted() {
    let api = server(false);
    api.reply(
        CORE,
        200,
        list(vec![
            core_event("x", "web-0", T1, 1),
            json!({"metadata": {"name": "broken", "uid": "b"}, "reason": "NoSubject"}),
        ]),
    );
    let mut feed = events(
        &api,
        EventsConfig {
            apis: EventApis::CoreOnly,
            ..EventsConfig::default()
        },
    )
    .watch(&cluster_scope(), &EventsOptions::default())
    .await
    .unwrap();
    assert_eq!(restart_uids(&next_batch(&mut feed).await), ["x"]);
    assert_eq!(feed.stats().skipped, 1);
}

#[tokio::test(start_paused = true)]
async fn bad_scopes_are_validation_errors() {
    let api = server(true);
    for scope in [
        WatchScope::Namespaces(vec![]),
        WatchScope::Namespaces(vec![String::new()]),
    ] {
        let err = events(&api, EventsConfig::default())
            .watch(&scope, &EventsOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
    }
}

#[tokio::test(start_paused = true)]
async fn nothing_is_requested_until_a_consumer_opens_a_feed_and_dropping_stops_it() {
    let api = server(false);
    api.reply(CORE, 200, list(vec![core_event("x", "web-0", T1, 1)]));
    let kube_events = events(&api, EventsConfig::default());
    settle().await;
    assert_eq!(api.hits(CORE), 0, "no events request without a subscriber");

    let feed = kube_events
        .watch(&cluster_scope(), &EventsOptions::default())
        .await
        .unwrap();
    let state = feed.state();
    settle().await;
    assert!(api.hits(CORE) >= 1);
    drop(feed);
    settle().await;
    assert_eq!(
        *state.borrow(),
        FeedState::Stopped,
        "the feed task ended with the handle"
    );
}
