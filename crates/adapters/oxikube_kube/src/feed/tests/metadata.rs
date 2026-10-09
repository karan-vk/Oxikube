//! Metadata-only feeds: the `PartialObjectMetadata` requests, partial objects, and upgrading
//! one object to a full one without disturbing the running feed.

use oxikube_domain::ErrorKind;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{ResourceReader, WatchOptions};

use super::server::*;
use super::*;
use crate::feed::FeedState;

fn default_ns() -> WatchScope {
    WatchScope::Namespaces(vec!["default".into()])
}

fn metadata_only() -> WatchOptions {
    WatchOptions::default().metadata_only()
}

/// Whether `accept` asks for `PartialObjectMetadata` (a list or an item), as kube sends it.
fn asks_for_metadata(accept: &Option<String>) -> bool {
    accept.as_deref().is_some_and(|a| {
        a.contains("as=PartialObjectMetadata") && a.contains("g=meta.k8s.io") && a.contains("v=v1")
    })
}

/// Watch requests made to `path` so far.
fn watches(server: &FeedServer, path: &str) -> usize {
    server
        .queries(path)
        .iter()
        .filter(|q| q.contains("watch=true"))
        .count()
}

/// A full pod as a `get` reply: typed, with a spec.
fn full_pod(name: &str, uid: &str, rv: &str) -> serde_json::Value {
    typed_pod(pod(name, uid, rv))
}

#[tokio::test(start_paused = true)]
async fn requests_ask_for_partial_object_metadata_and_objects_are_partial() {
    let server = FeedServer::new(31);
    server.list(
        PODS,
        pod_list(
            vec![meta_pod("a", "ua", "1"), meta_pod("b", "ub", "2")],
            "10",
        ),
    );
    server.watch(
        PODS,
        Reply::Events(vec![
            meta_event("MODIFIED", meta_pod("a", "ua", "11")),
            meta_event("ADDED", meta_pod("c", "uc", "12")),
            meta_event("DELETED", meta_pod("b", "ub", "13")),
        ]),
    );
    let mut feed = server
        .resources(config())
        .metadata_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();
    assert!(feed.is_metadata_only());

    let first = next_batch(&mut feed).await;
    assert_eq!(labels(&first), vec!["restart[a,b]"]);
    let second = next_batch(&mut feed).await;
    assert_eq!(labels(&second), vec!["+a@11", "+c@12", "-b@13"]);

    // Every object is marked partial, whichever way it arrived.
    let Delta::Restarted(listed) = &first.deltas[0] else {
        panic!("the first delta is the list");
    };
    let changed = second.deltas.iter().map(|d| match d {
        Delta::Applied(r) | Delta::Deleted(r) => r,
        Delta::Restarted(_) => unreachable!(),
    });
    for resource in listed.iter().chain(changed) {
        assert!(resource.is_partial(), "{resource:?}");
        // The kind is the discovered one, not the server's `PartialObjectMetadata`.
        assert_eq!(resource.kind, pod_gvk());
        assert_eq!(resource.to_value()["kind"], "Pod");
        assert_eq!(resource.to_value()["apiVersion"], "v1");
        assert!(resource.get("/spec").is_none() && resource.get("/status").is_none());
        assert!(resource.get("/metadata/managedFields").is_none());
        assert_eq!(resource.meta.labels.get("app").map(|v| &**v), Some("web"));
        assert_eq!(resource.meta.owner_refs[0].name.as_ref(), "web-1");
    }
    let stored = feed.snapshot();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|o| o.is_partial()));

    // The list and the watch both carried the PartialObjectMetadata Accept header.
    let accepts = server.accepts(PODS);
    assert!(accepts.iter().any(|(watch, _)| !watch), "a list was made");
    assert!(accepts.iter().any(|(watch, _)| *watch), "a watch was made");
    for (watch, accept) in &accepts {
        assert!(
            asks_for_metadata(accept),
            "{} request accepted {accept:?}",
            if *watch { "watch" } else { "list" }
        );
    }
    assert_eq!(*feed.state().borrow(), FeedState::Live);
}

#[tokio::test(start_paused = true)]
async fn a_full_feed_asks_for_full_objects_and_they_are_not_partial() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![pod("a", "ua", "1")], "10"));
    let mut feed = server
        .resources(config())
        .reflector_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();
    assert!(!feed.is_metadata_only());
    next_batch(&mut feed).await;

    assert!(feed.snapshot().iter().all(|o| !o.is_partial()));
    assert!(
        server
            .accepts(PODS)
            .iter()
            .all(|(_, accept)| !asks_for_metadata(accept))
    );
}

#[tokio::test(start_paused = true)]
async fn the_port_watch_honours_metadata_only() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![meta_pod("a", "ua", "1")], "10"));
    let mut feed = server
        .resources(config())
        .watch(&pod_gvk(), Some("default"), &metadata_only())
        .await
        .unwrap();
    let batch = tokio::time::timeout(Duration::from_secs(600), feed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Delta::Restarted(listed) = &batch.deltas[0] else {
        panic!("the first delta is the list");
    };
    assert!(listed[0].is_partial());
}

#[tokio::test(start_paused = true)]
async fn a_streaming_metadata_feed_sends_the_metadata_accept_header_on_its_watch() {
    let server = FeedServer::new(32);
    server.watch(
        PODS,
        Reply::Events(vec![
            meta_event("ADDED", meta_pod("a", "ua", "5")),
            initial_events_end("7"),
        ]),
    );
    let mut feed = server
        .resources(FeedConfig {
            streaming_lists: StreamingLists::Auto,
            ..config()
        })
        .metadata_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();

    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a]"]);
    assert!(feed.snapshot()[0].is_partial());
    assert_eq!(server.list_hits(PODS), 0, "no LIST request");
    let accepts = server.accepts(PODS);
    assert!(
        accepts[0].0 && asks_for_metadata(&accepts[0].1),
        "{accepts:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn selectors_and_scopes_apply_to_metadata_feeds_too() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![meta_pod("a", "ua", "1")], "10"));
    server.list(
        OTHER_PODS,
        pod_list(vec![meta_pod_in("other", "b", "ub", "2")], "10"),
    );
    let scope = WatchScope::Namespaces(vec!["default".into(), "other".into()]);
    let mut feed = server
        .resources(config())
        .metadata_feed(
            &pod_gvk(),
            &scope,
            &WatchOptions::default().labels("app=web"),
        )
        .await
        .unwrap();
    assert_eq!(labels(&next_batch(&mut feed).await), vec!["restart[a,b]"]);
    for path in [PODS, OTHER_PODS] {
        assert!(
            server
                .queries(path)
                .iter()
                .all(|q| q.contains("labelSelector=app%3Dweb")),
            "{path}: {:?}",
            server.queries(path)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn upgrade_returns_the_full_object_and_leaves_the_feed_running() {
    let server = FeedServer::new(31);
    server.list(PODS, pod_list(vec![meta_pod("a", "ua", "1")], "10"));
    server.watch(
        PODS,
        Reply::Events(vec![meta_event("ADDED", meta_pod("b", "ub", "11"))]),
    );
    // `get` of one pod: a plain object at its own path.
    server.list(
        &format!("{PODS}/a"),
        Reply::Json(200, full_pod("a", "ua", "1")),
    );

    let resources = server.resources(config());
    let mut feed = resources
        .metadata_feed(&pod_gvk(), &default_ns(), &WatchOptions::default())
        .await
        .unwrap();
    let Delta::Restarted(listed) = next_batch(&mut feed).await.deltas.remove(0) else {
        panic!("the first delta is the list");
    };
    let partial = listed.into_iter().next().unwrap();
    assert!(partial.is_partial());
    assert!(partial.get("/spec").is_none());

    let watches_before = watches(&server, PODS);
    let open_before = server.open_watches();
    let upgraded = resources.upgrade(&partial).await.unwrap();

    assert!(!upgraded.is_partial());
    assert_eq!(upgraded.name(), "a");
    assert_eq!(upgraded.meta.uid, partial.meta.uid);
    assert_eq!(
        upgraded.get_str("/spec/containers/0/image"),
        Some("busybox"),
        "the full object has its spec"
    );
    // One GET of the object, and nothing else: no new watch, the running one still open.
    let get_accepts = server.accepts(&format!("{PODS}/a"));
    assert_eq!(get_accepts.len(), 1);
    assert!(!get_accepts[0].0 && !asks_for_metadata(&get_accepts[0].1));
    assert_eq!(watches(&server, PODS), watches_before, "no feed started");
    assert_eq!(server.open_watches(), open_before);

    // The feed carries on, still metadata-only, and its store was not touched.
    assert_eq!(labels(&next_batch(&mut feed).await), vec!["+b@11"]);
    assert!(!feed.is_finished());
    let stored = feed.snapshot();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|o| o.is_partial()));
}

#[tokio::test(start_paused = true)]
async fn upgrade_of_a_gone_or_replaced_object_is_not_found() {
    let server = FeedServer::new(31);
    server.list(
        &format!("{PODS}/a"),
        Reply::Json(200, full_pod("a", "other-uid", "9")),
    );
    let resources = server.resources(config());
    let partial = |name: &str, uid: &str| {
        let object = typed_pod(meta_pod(name, uid, "1"));
        Resource::from_json(object).unwrap().into_partial()
    };

    // The name now belongs to a different object.
    let err = resources.upgrade(&partial("a", "ua")).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("replaced"), "{}", err.message());

    // The object is gone (no reply scripted: the server answers 404).
    let err = resources.upgrade(&partial("gone", "ug")).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}
