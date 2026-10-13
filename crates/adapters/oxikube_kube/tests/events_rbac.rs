//! Kind integration for E14-S01: the events watcher under namespace-scoped RBAC. A service
//! account that may list and watch `events` (core group) in one namespace only: the
//! cluster-wide watch of both event APIs is refused, the feed falls back to the namespaces the
//! caller names, shows the readable one and reports the other as forbidden. Needs
//! `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{Event as CoreEvent, ObjectReference};
use k8s_openapi::api::rbac::v1::PolicyRule;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::Api;
use kube::api::{ObjectMeta, PostParams};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::WatchScope;
use oxikube_kube::{EventApi, EventsConfig, EventsOptions, KubeEvents};
use oxikube_ports::{Delta, ForbiddenScope};
use oxikube_testkit::integration::TestNamespace;

use common::resources::adapter;
use common::{DEADLINE, TestServiceAccount};

fn event_in(namespace: &str) -> CoreEvent {
    CoreEvent {
        metadata: ObjectMeta {
            generate_name: Some("oxi-rbac-".into()),
            namespace: Some(namespace.to_owned()),
            ..ObjectMeta::default()
        },
        involved_object: ObjectReference {
            kind: Some("Namespace".into()),
            name: Some(namespace.to_owned()),
            namespace: Some(namespace.to_owned()),
            api_version: Some("v1".into()),
            ..ObjectReference::default()
        },
        reason: Some("OxiRbac".into()),
        message: Some("visible to the namespace's reader".into()),
        type_: Some("Warning".into()),
        first_timestamp: Some(Time(jiff::Timestamp::now())),
        last_timestamp: Some(Time(jiff::Timestamp::now())),
        count: Some(1),
        ..CoreEvent::default()
    }
}

#[tokio::test]
async fn a_namespace_reader_gets_its_events_and_the_other_namespace_is_reported_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let readable = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let hidden = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let events_in_core = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["events".into()]),
        verbs: vec!["get".into(), "list".into(), "watch".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, readable.name(), "oxi-events", vec![events_in_core])
            .await;
    for ns in [&readable, &hidden] {
        Api::<CoreEvent>::namespaced((*admin).clone(), ns.name())
            .create(&PostParams::default(), &event_in(ns.name()))
            .await
            .expect("create an event");
    }

    let context = ContextName::from("oxi-events");
    let pool = kind.pool(kind.with_token_context(context.as_str(), &account.token));
    let client = pool.get(&context).await.expect("the reader's client");
    let events = KubeEvents::new(adapter(&client), ClusterId::new("kind-test", &kind.context))
        .with_config(EventsConfig::default());
    let options = EventsOptions::default()
        .fallback_namespaces([readable.name().to_owned(), hidden.name().to_owned()]);

    // RBAC takes a moment to apply: the first attempts may see everything forbidden.
    let deadline = tokio::time::Instant::now() + DEADLINE * 2;
    let feed = loop {
        let mut feed = events
            .watch(&WatchScope::Cluster, &options)
            .await
            .expect("open the feed");
        match tokio::time::timeout_at(deadline, feed.next()).await {
            Ok(Some(Ok(batch))) => {
                let Some(Delta::Restarted(all)) = batch.deltas.first() else {
                    panic!("the feed opens with a Restarted");
                };
                assert!(all.iter().any(|e| &*e.reason == "OxiRbac"), "{all:?}");
                break feed;
            }
            Ok(Some(Err(_)) | None) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            other => panic!("the reader's feed did not open: {other:?}"),
        }
    };
    assert_eq!(
        feed.apis(),
        [EventApi::Core],
        "the only API the reader may use"
    );
    let want = [ForbiddenScope::Namespace(hidden.name().to_owned())];
    let started = tokio::time::Instant::now();
    while feed.forbidden() != want {
        assert!(
            started.elapsed() < DEADLINE,
            "forbidden: {:?}",
            feed.forbidden()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !feed.is_finished(),
        "the readable namespace keeps the feed alive"
    );
}
