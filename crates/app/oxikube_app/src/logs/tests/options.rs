//! What reaches the port (follow, since, tail, previous, timestamps, container) and how a target
//! is keyed.

use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{LogOptions, LogSince};
use oxikube_testkit::LogCall;

use super::{Harness, burst};
use crate::logs::LogTarget;

fn only_call(h: &Harness) -> (String, String, LogOptions) {
    match h.port.recorded_calls().as_slice() {
        [
            LogCall::StreamLogs {
                namespace,
                pod,
                options,
            },
        ] => (namespace.clone(), pod.clone(), options.clone()),
        other => panic!("expected one stream_logs call, got {other:?}"),
    }
}

#[test]
fn every_option_reaches_the_port_unchanged() {
    let mut h = Harness::new();
    let options = LogOptions::follow()
        .tail_lines(200)
        .since(LogSince::Seconds(300))
        .limit_bytes(1 << 20)
        .previous();
    let _session = h.open(
        burst(1).keep_open(),
        LogTarget::pod("shop", "web-0"),
        options.clone(),
    );
    let (namespace, pod, passed) = only_call(&h);
    assert_eq!((namespace.as_str(), pod.as_str()), ("shop", "web-0"));
    assert_eq!(passed, options);
    assert!(passed.follow && passed.previous && passed.timestamps);
    assert_eq!(passed.tail_lines, Some(200));
    assert_eq!(passed.since, Some(LogSince::Seconds(300)));
}

#[test]
fn since_a_time_and_plain_options_pass_through_too() {
    let mut h = Harness::new();
    let at = super::ts(0);
    let options = LogOptions::default().since(LogSince::Time(at));
    let _session = h.open(burst(1), LogTarget::pod("default", "web-0"), options);
    let (_, _, passed) = only_call(&h);
    assert_eq!(passed.since, Some(LogSince::Time(at)));
    assert!(!passed.follow && !passed.previous && !passed.timestamps);
}

#[test]
fn the_targets_container_is_the_one_read() {
    let mut h = Harness::new();
    let session = h.open(
        burst(1),
        LogTarget::pod("default", "web-0").container("sidecar"),
        LogOptions::default(),
    );
    let (_, _, passed) = only_call(&h);
    assert_eq!(passed.container.as_deref(), Some("sidecar"));
    assert_eq!(session.target().container.as_deref(), Some("sidecar"));
    assert_eq!(session.options().container.as_deref(), Some("sidecar"));
}

#[test]
fn a_container_in_the_options_names_the_target_when_it_has_none() {
    let mut h = Harness::new();
    let session = h.open(
        burst(1),
        LogTarget::pod("default", "web-0"),
        LogOptions::default().container("app"),
    );
    assert_eq!(session.target().container.as_deref(), Some("app"));
    let (_, _, passed) = only_call(&h);
    assert_eq!(passed.container.as_deref(), Some("app"));
}

#[test]
fn the_target_wins_over_a_different_container_in_the_options() {
    let mut h = Harness::new();
    let session = h.open(
        burst(1),
        LogTarget::pod("default", "web-0").container("a"),
        LogOptions::default().container("b"),
    );
    assert_eq!(session.options().container.as_deref(), Some("a"));
}

#[test]
fn a_target_is_a_pod_and_optionally_a_container() {
    let pod = LogTarget::pod("shop", "web-0");
    assert_eq!(pod.to_string(), "shop/web-0");
    assert_eq!(pod.clone().container("app").to_string(), "shop/web-0/app");
    assert_ne!(pod, LogTarget::pod("shop", "web-0").container("app"));

    let cluster = ClusterId::new("kubeconfig", &ContextName::new("kind"));
    let reference =
        ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    assert_eq!(LogTarget::of(&reference), Some(pod));
    let node = ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Node"), "n1");
    assert_eq!(LogTarget::of(&node), None);
}

#[test]
fn open_sessions_are_listed_and_found_by_target() {
    let mut h = Harness::new();
    let a = h.open(
        burst(1).keep_open(),
        LogTarget::pod("default", "a"),
        LogOptions::follow(),
    );
    let b = h.open(
        burst(1).keep_open(),
        LogTarget::pod("default", "b"),
        LogOptions::follow(),
    );
    let listed: Vec<_> = h
        .service
        .sessions()
        .iter()
        .map(|r| r.target().to_string())
        .collect();
    assert_eq!(listed, ["default/a", "default/b"]);
    assert_eq!(
        h.service
            .reader_for(&LogTarget::pod("default", "b"))
            .unwrap()
            .id(),
        b.id()
    );
    assert!(
        h.service
            .reader_for(&LogTarget::pod("default", "c"))
            .is_none()
    );
    assert_ne!(a.id(), b.id());

    drop(a);
    assert_eq!(h.service.sessions().len(), 1);
    assert!(
        h.service
            .reader_for(&LogTarget::pod("default", "a"))
            .is_none()
    );
}

#[test]
fn a_reader_does_not_keep_the_stream_open() {
    let mut h = Harness::new();
    let session = h.follow_flushed(burst(1).keep_open());
    let reader = session.reader();
    drop(session);
    h.settle();
    assert_eq!(h.port.live_streams(), 0);
    assert_eq!(reader.len(), 1);
}
