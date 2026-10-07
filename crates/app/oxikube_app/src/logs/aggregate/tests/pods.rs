//! Resolving the selector, following the pod set, the stream cap and the failures.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::LogOptions;
use oxikube_testkit::{ResourceCall, ScriptedFeed, TICK, Timeline, pod};

use super::{Harness, line, texts, web, web_pod, web_ref};
use crate::logs::{AggregateSpec, LogConfig, LogState, PodChange};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn quiet() -> Timeline<oxikube_domain::log::LogLine> {
    Timeline::new().keep_open()
}

#[test]
fn a_deployments_selector_picks_the_pods_that_are_streamed_with_timestamps() {
    let mut h = Harness::new();
    let other = pod()
        .name("db-0")
        .namespace("default")
        .label("app", "db")
        .build();
    h.seed([web(), web_pod("web-a"), web_pod("web-b"), other]);
    h.script(quiet());
    h.script(quiet());
    let session = h.open_web();
    assert_eq!(session.aggregate().label(), "deployment/web");
    assert_eq!(session.aggregate().selector().as_deref(), Some("app=web"));

    let streams: Vec<_> = h
        .logs
        .recorded_calls()
        .into_iter()
        .map(|call| match call {
            oxikube_testkit::LogCall::StreamLogs {
                namespace,
                pod,
                options,
            } => (
                namespace,
                pod,
                options.container,
                options.timestamps,
                options.follow,
            ),
        })
        .collect();
    assert_eq!(
        streams,
        [
            (
                "default".into(),
                "web-a".into(),
                Some("app".into()),
                true,
                true
            ),
            (
                "default".into(),
                "web-b".into(),
                Some("app".into()),
                true,
                true
            ),
        ],
        "one stream per container of the matching pods only, with server timestamps"
    );
    let watches: Vec<_> = h
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Watch {
                kind,
                namespace,
                options,
            } => Some((kind.kind.to_string(), namespace, options.label_selector)),
            _ => None,
        })
        .collect();
    assert_eq!(
        watches,
        [(
            "Pod".to_string(),
            Some("default".to_string()),
            Some("app=web".to_string())
        )]
    );
    assert_eq!(session.state(), LogState::Streaming);
}

#[test]
fn a_service_and_a_typed_selector_resolve_too_and_a_narrowing_selector_is_anded() {
    let mut h = Harness::new();
    let service = oxikube_testkit::resource("v1", "Service")
        .name("web-svc")
        .namespace("default")
        .field("spec", serde_json::json!({"selector": {"app": "web"}}))
        .build();
    let canary = pod()
        .name("web-canary")
        .namespace("default")
        .label("app", "web")
        .label("track", "canary")
        .build();
    h.seed([service, web_pod("web-a"), canary]);

    let svc = ResourceRef::namespaced(
        ClusterId::new("~/.kube/config", &ContextName::new("kind")),
        Gvk::new("", "v1", "Service"),
        "default",
        "web-svc",
    );
    h.script(quiet());
    h.script(quiet());
    let session = h.open(AggregateSpec::of(&svc).unwrap(), LogOptions::follow());
    assert_eq!(session.aggregate().label(), "service/web-svc");
    assert_eq!(h.logs.recorded_calls().len(), 2, "both pods of the Service");
    drop(session);

    let mut h2 = Harness::new();
    h2.seed([web_pod("web-a"), web_pod("web-b")]);
    h2.script(quiet());
    let spec = AggregateSpec::selector("default", "app=web").container("app");
    let session = h2.open(spec, LogOptions::follow());
    assert_eq!(session.aggregate().label(), "selector app=web");
    assert_eq!(
        h2.logs.recorded_calls().len(),
        2,
        "a typed selector needs no object"
    );

    let mut h3 = Harness::new();
    h3.seed([
        web_pod("web-a"),
        pod()
            .name("web-canary")
            .namespace("default")
            .label("app", "web")
            .label("track", "canary")
            .build(),
    ]);
    h3.script(quiet());
    let spec = AggregateSpec::selector("default", "app=web").also_matching("track=canary");
    let session = h3.open(spec, LogOptions::follow());
    assert_eq!(
        session.aggregate().selector().as_deref(),
        Some("app=web,track=canary")
    );
    let calls = h3.logs.recorded_calls();
    assert_eq!(calls.len(), 1);
}

#[test]
fn a_pod_that_appears_or_goes_away_is_an_event_and_the_baseline_is_not() {
    let mut h = Harness::new();
    h.seed([web()]);
    ScriptedFeed::new()
        .initial([web_pod("web-a"), web_pod("web-b")])
        .add(1, web_pod("web-c"))
        .delete(2, web_pod("web-a"))
        .install(&h.resources);
    for _ in 0..3 {
        h.script(quiet());
    }
    let session = h.open_web();
    let view = session.aggregate().clone();
    let mut changes = view.changes();
    assert!(
        view.events_after(None).is_empty(),
        "the pods of the first list are the baseline"
    );
    assert_eq!(view.sources().len(), 2);

    h.run_for(TICK);
    let events = view.events_after(None);
    assert_eq!(events.len(), 1);
    assert_eq!(
        (&*events[0].pod, events[0].change),
        ("web-c", PodChange::Added)
    );
    assert_eq!(view.sources().len(), 3, "the new pod is streamed");

    h.run_for(TICK);
    let events = view.events_after(Some(events[0].seq));
    assert_eq!(events.len(), 1);
    assert_eq!(
        (&*events[0].pod, events[0].change),
        ("web-a", PodChange::Ended)
    );

    // The change stream fired (coalesced) so a viewer wakes for the banner.
    use futures::StreamExt as _;
    let fired = futures::executor::block_on(async { changes.next().await });
    assert!(fired.is_some());
}

#[test]
fn the_stream_cap_leaves_pods_out_and_says_how_many() {
    let mut h = Harness::with_config(LogConfig {
        max_streams: 2,
        ..LogConfig::default()
    });
    h.seed([
        web(),
        web_pod("web-a"),
        web_pod("web-b"),
        web_pod("web-c"),
        web_pod("web-d"),
    ]);
    for _ in 0..4 {
        h.script(quiet());
    }
    let session = h.open_web();
    let view = session.aggregate();
    let streaming: Vec<String> = view.sources().iter().map(|s| s.pod.to_string()).collect();
    assert_eq!(streaming, ["web-a", "web-b"]);
    assert_eq!(
        view.skipped_pods(),
        2,
        "the 'N more pods not streamed' notice"
    );
    assert_eq!(h.logs.live_streams(), 2, "bounded concurrency");

    // Raising the setting starts the pods that were left out, without reopening anything.
    h.service.set_max_streams(10);
    h.run_for(Duration::from_secs(2));
    assert_eq!(view.sources().len(), 4);
    assert_eq!(view.skipped_pods(), 0);
    assert_eq!(h.logs.live_streams(), 4);
}

#[test]
fn a_line_after_a_quiet_spell_is_not_held_back_by_the_cap_recheck() {
    let mut h = Harness::with_config(LogConfig {
        max_streams: 1,
        ..LogConfig::default()
    });
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    // The one stream is idle, a pod waits for it: the loop sleeps on the slow cap recheck (1 s).
    h.script(
        Timeline::new()
            .ok_at(ms(100), line("web-a", 100, "late"))
            .keep_open(),
    );
    let session = h.open_web();
    assert_eq!(session.aggregate().skipped_pods(), 1);
    // The reorder window (300 ms) and a flush tick later the line shows, long before the recheck.
    h.run_for(ms(600));
    assert_eq!(texts(&session), ["late"]);
}

#[test]
fn a_container_that_has_not_started_is_read_once_its_pod_says_so() {
    let mut h = Harness::new();
    h.seed([web()]);
    let creating = pod()
        .name("web-a")
        .namespace("default")
        .label("app", "web")
        .container_creating();
    ScriptedFeed::new()
        .initial([creating.build()])
        .modify(1, web_pod("web-a"))
        .install(&h.resources);
    h.script(Timeline::immediate([line("web-a", 1, "hello")]).keep_open());
    let session = h.open_web();
    assert!(
        session.aggregate().sources().is_empty(),
        "nothing to read yet"
    );
    h.run_for(TICK);
    h.run_for(ms(600));
    assert_eq!(texts(&session), ["hello"]);
}

#[test]
fn failures_are_the_sessions_state_and_a_failing_pod_does_not_stop_the_others() {
    // The object does not exist.
    let mut h = Harness::new();
    let session = h.open_web();
    let LogState::Failed(failure) = session.state() else {
        panic!("{:?}", session.state());
    };
    assert_eq!(failure.kind, ErrorKind::NotFound);

    // The object has no selector.
    let mut h = Harness::new();
    let service = oxikube_testkit::resource("v1", "Service")
        .name("web")
        .namespace("default")
        .field("spec", serde_json::json!({"ports": []}))
        .build();
    h.seed([service]);
    let svc = ResourceRef::namespaced(
        web_ref().cluster,
        Gvk::new("", "v1", "Service"),
        "default",
        "web",
    );
    let session = h.open(AggregateSpec::of(&svc).unwrap(), LogOptions::follow());
    let LogState::Failed(failure) = session.state() else {
        panic!("{:?}", session.state());
    };
    assert_eq!(failure.kind, ErrorKind::Validation);
    assert!(
        failure.message.contains("no selector"),
        "{}",
        failure.message
    );

    // One pod's stream cannot be opened: that source fails, the other keeps streaming.
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.logs
        .script()
        .stream_logs
        .push_err(oxikube_domain::OxiError::forbidden("pods/log is forbidden"));
    h.script(Timeline::immediate([line("web-b", 1, "b-line")]).keep_open());
    let session = h.open_web();
    h.run_for(ms(600));
    let sources = session.aggregate().sources();
    assert!(matches!(
        sources[0].state,
        crate::logs::SourceState::Failed(_)
    ));
    assert!(sources[1].state.is_live());
    assert_eq!(texts(&session), ["b-line"]);
    assert_eq!(session.state(), LogState::Streaming);
}
