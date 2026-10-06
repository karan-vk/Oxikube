//! Kind integration for E04-S12: the events feed against a real API server. A pod with a bad
//! image produces a `Warning` event on both event APIs; the merged feed holds each event
//! once, the per-object feed holds only that object's events, and a storm of events stays
//! within the ring's capacity. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips
//! cleanly otherwise.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace and the feeds are scoped to
//! it, so concurrent suites do not see each other's events.
#![cfg(feature = "integration")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use futures::StreamExt;
use jiff::Timestamp;
use k8s_openapi::api::core::v1::{Container, Event as CoreEvent, ObjectReference, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::Api;
use kube::Client;
use kube::api::{ListParams, ObjectMeta, PostParams};
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::WatchScope;
use oxikube_kube::{EventApi, EventApis, EventFeed, EventsConfig, EventsOptions, KubeEvents};
use oxikube_ports::Delta;
use oxikube_testkit::integration::TestNamespace;

use common::resources::{adapter, unschedulable_pod};
use common::{DEADLINE, Kind};

/// The events of an `oxikube.invalid` image: the pull fails at once, with a `Warning`.
fn broken_pod(name: &str) -> Pod {
    Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            termination_grace_period_seconds: Some(1),
            containers: vec![Container {
                name: "broken".into(),
                image: Some("oxikube.invalid/does-not-exist:1".into()),
                ..Container::default()
            }],
            ..PodSpec::default()
        }),
        ..Pod::default()
    }
}

fn kube_events(client: &Client, kind: &Kind, config: EventsConfig) -> KubeEvents {
    KubeEvents::new(adapter(client), ClusterId::new("kind-test", &kind.context)).with_config(config)
}

/// What a consumer folding the feed's deltas holds, by event uid.
#[derive(Default)]
struct Folded(BTreeMap<String, Event>);

impl Folded {
    fn apply(&mut self, delta: Delta<Event>) {
        let uid = |e: &Event| {
            e.uid
                .as_deref()
                .expect("a server event has a uid")
                .to_owned()
        };
        match delta {
            Delta::Restarted(all) => self.0 = all.into_iter().map(|e| (uid(&e), e)).collect(),
            Delta::Applied(e) => {
                self.0.insert(uid(&e), e);
            }
            Delta::Deleted(e) => {
                self.0.remove(&uid(&e));
            }
        }
    }

    /// Folds items until `done` holds, failing after [`DEADLINE`].
    async fn until(&mut self, feed: &mut EventFeed, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + DEADLINE * 2;
        while !done(self) {
            let item = tokio::time::timeout_at(deadline, feed.next())
                .await
                .unwrap_or_else(|_| panic!("{what} not reached in time"))
                .expect("the feed is open")
                .expect("no feed error");
            for delta in item {
                self.apply(delta);
            }
        }
    }
}

fn scope(ns: &TestNamespace) -> WatchScope {
    WatchScope::Namespaces(vec![ns.name().to_owned()])
}

/// A failing pod yields a Warning on both APIs, merged to one event each; the per-object
/// feed holds exactly that pod's events.
#[tokio::test]
async fn a_failing_pod_has_a_warning_and_the_object_feed_isolates_it() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
    // Two objects with warnings: the broken one (image pull) and an unschedulable one
    // (FailedScheduling), so the filter has something to exclude.
    let broken = pods
        .create(&PostParams::default(), &broken_pod("broken"))
        .await
        .expect("create the broken pod");
    pods.create(&PostParams::default(), &unschedulable_pod("stuck", &[]))
        .await
        .expect("create the unschedulable pod");
    let broken_uid = broken.metadata.uid.expect("pod uid");

    // The namespace feed: both APIs, until both pods have a Warning.
    let events = kube_events(&client, &kind, EventsConfig::default());
    let mut feed = events
        .watch(&scope(&ns), &EventsOptions::default())
        .await
        .expect("open the namespace feed");
    assert_eq!(feed.apis(), [EventApi::Core, EventApi::EventsV1]);
    let mut all = Folded::default();
    all.until(&mut feed, "a Warning for each pod", |f| {
        ["broken", "stuck"].iter().all(|pod| {
            f.0.values()
                .any(|e| e.event_type == EventType::Warning && &*e.regarding.name == *pod)
        })
    })
    .await;

    // Merged: the folded set is exactly the set of distinct events the server holds (the
    // core API lists every stored event once; the same ones come from events.k8s.io).
    let core = Api::<CoreEvent>::namespaced((*client).clone(), ns.name());
    let started = std::time::Instant::now();
    let stored = loop {
        // New events keep arriving while the pods fail; compare repeatedly.
        let fresh: BTreeSet<String> = core
            .list(&ListParams::default())
            .await
            .expect("list events")
            .items
            .into_iter()
            .filter_map(|e| e.metadata.uid)
            .collect();
        while let Ok(Some(item)) =
            tokio::time::timeout(Duration::from_millis(500), feed.next()).await
        {
            item.expect("no feed error")
                .into_iter()
                .for_each(|d| all.apply(d));
        }
        let folded: BTreeSet<String> = all.0.keys().cloned().collect();
        if folded == fresh {
            break fresh;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "the feed holds {} events, the server {}",
            folded.len(),
            fresh.len()
        );
    };
    assert!(stored.len() >= 2);
    assert_eq!(
        all.0.len(),
        stored.len(),
        "each event once, not once per API"
    );
    let warning = all
        .0
        .values()
        .find(|e| e.event_type == EventType::Warning && &*e.regarding.name == "broken")
        .expect("the broken pod's warning");
    assert_eq!(&*warning.regarding.gvk.kind, "Pod");
    assert_eq!(warning.regarding_uid.as_deref(), Some(broken_uid.as_str()));
    assert!(warning.last_seen.is_some() && warning.count >= 1);
    assert!(!warning.message.is_empty());
    drop(feed);

    // The per-object feed, on both APIs: only the broken pod's events, Warning included.
    let mut object_feed = events
        .watch(&scope(&ns), &EventsOptions::for_object(broken_uid.clone()))
        .await
        .expect("open the object feed");
    let mut mine = Folded::default();
    mine.until(&mut object_feed, "the broken pod's Warning", |f| {
        f.0.values().any(Event::is_warning)
    })
    .await;
    assert!(
        mine.0
            .values()
            .all(|e| e.regarding_uid.as_deref() == Some(broken_uid.as_str())),
        "only the filtered object's events: {:?}",
        mine.0
            .values()
            .map(|e| &e.regarding.name)
            .collect::<Vec<_>>()
    );
    // The other pod's events exist (the namespace feed saw them) and none leaked in.
    let others: BTreeSet<&String> = all
        .0
        .iter()
        .filter(|(_, e)| e.regarding_uid.as_deref() != Some(broken_uid.as_str()))
        .map(|(uid, _)| uid)
        .collect();
    assert!(!others.is_empty(), "the unschedulable pod has events");
    assert!(mine.0.keys().all(|uid| !others.contains(uid)));
    assert_eq!(object_feed.stats().skipped, 0);
}

/// Each API alone yields the same events as both merged.
#[tokio::test]
async fn each_api_alone_agrees_with_the_merged_feed() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
    pods.create(&PostParams::default(), &unschedulable_pod("stuck", &[]))
        .await
        .expect("create the unschedulable pod");

    let ready = |f: &Folded| f.0.values().any(Event::is_warning);
    let mut sets = Vec::new();
    for apis in [
        EventApis::Both,
        EventApis::CoreOnly,
        EventApis::EventsV1Only,
    ] {
        let mut feed = kube_events(
            &client,
            &kind,
            EventsConfig {
                apis,
                ..EventsConfig::default()
            },
        )
        .watch(&scope(&ns), &EventsOptions::default())
        .await
        .expect("open the feed");
        let mut folded = Folded::default();
        folded.until(&mut feed, "a Warning", ready).await;
        // The scheduler may add or update events; the stable part is who they are about.
        let reasons: BTreeSet<_> = folded
            .0
            .values()
            .map(|e| (e.regarding.name.to_string(), e.reason.to_string()))
            .collect();
        sets.push(reasons);
    }
    assert_eq!(sets[0], sets[1], "merged vs core/v1");
    assert_eq!(sets[0], sets[2], "merged vs events.k8s.io/v1");
}

/// A storm of distinct events never holds more than the capacity and the newest survive.
#[tokio::test]
async fn a_storm_of_events_stays_within_capacity() {
    const CAPACITY: usize = 20;
    const STORM: usize = 70;
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let core = Api::<CoreEvent>::namespaced((*client).clone(), ns.name());
    let base = Timestamp::now();

    let config = EventsConfig {
        capacity: CAPACITY,
        ..EventsConfig::default()
    };
    let mut feed = kube_events(&client, &kind, config)
        .watch(&scope(&ns), &EventsOptions::default())
        .await
        .expect("open the feed");
    let mut folded = Folded::default();
    folded.until(&mut feed, "the opening list", |_| true).await;

    let writer = async {
        for i in 0..STORM {
            let at = Time(base + Duration::from_secs(i as u64));
            let event = CoreEvent {
                metadata: ObjectMeta {
                    name: Some(format!("storm-{i:03}")),
                    ..ObjectMeta::default()
                },
                involved_object: ObjectReference {
                    kind: Some("Pod".into()),
                    name: Some("noisy".into()),
                    namespace: Some(ns.name().to_owned()),
                    uid: Some("00000000-0000-0000-0000-000000000001".into()),
                    api_version: Some("v1".into()),
                    ..ObjectReference::default()
                },
                reason: Some("Storm".into()),
                message: Some(format!("event {i}")),
                type_: Some("Normal".into()),
                first_timestamp: Some(at.clone()),
                last_timestamp: Some(at),
                count: Some(1),
                reporting_component: Some("oxikube-test".into()),
                reporting_instance: Some("oxikube-test-0".into()),
                ..CoreEvent::default()
            };
            core.create(&PostParams::default(), &event)
                .await
                .expect("create an event");
        }
    };
    let reader = async {
        let newest = format!("event {}", STORM - 1);
        folded
            .until(&mut feed, "the newest storm event", |f| {
                assert!(
                    f.0.len() <= CAPACITY,
                    "the consumer holds {} events",
                    f.0.len()
                );
                f.0.values().any(|e| e.message == newest)
            })
            .await;
    };
    tokio::join!(writer, reader);

    let stats = feed.stats();
    assert!(stats.len <= CAPACITY && folded.0.len() <= CAPACITY);
    assert_eq!(
        stats.evicted as usize,
        STORM - CAPACITY,
        "every event beyond the capacity was evicted and counted (no other events in this namespace)"
    );
    // What survived is the newest: the last CAPACITY storm events.
    let mut kept: Vec<_> = folded.0.values().map(|e| e.message.clone()).collect();
    kept.sort();
    let mut expected: Vec<_> = (STORM - CAPACITY..STORM)
        .map(|i| format!("event {i}"))
        .collect();
    expected.sort();
    assert_eq!(kept, expected);
}
