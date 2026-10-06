//! How the detail stands when the object is deleted, missing or cannot be listed.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;

use super::fixture::{Detail, pod_ref, web_pod};
use crate::detail::DetailState;

#[gpui::test]
fn a_deleted_object_says_so_and_keeps_its_last_known_state(cx: &mut TestAppContext) {
    let pod = web_pod();
    let mut d = Detail::new(cx, [pod.clone()]);
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            pod.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![Delta::Deleted(pod)]),
        )
        .keep_open(),
    );
    let view = d.open(&pod_ref("web-0"));
    assert_eq!(d.read(&view, |v| v.state().clone()), DetailState::Live);
    assert!(!d.shown("detail-banner"));

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    assert_eq!(
        d.read(&view, |v| v.state().clone()),
        DetailState::Deleted,
        "a delete from the feed marks the detail"
    );
    assert!(d.shown("detail-banner"), "a clear 'deleted' state");
    assert!(
        d.shown("detail-label-0") && d.shown("detail-name"),
        "the content does not blank: the last known state stays"
    );
    assert_eq!(
        d.read(&view, |v| v.model().map(|m| m.labels.len())),
        Some(2)
    );
}

#[gpui::test]
fn an_object_the_feed_does_not_have_is_not_found(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let view = d.open(&pod_ref("no-such-pod"));
    assert_eq!(d.read(&view, |v| v.state().clone()), DetailState::NotFound);
    assert!(d.shown("detail-banner"));
    assert!(d.read(&view, |v| v.model().is_none()));
}

#[gpui::test]
fn a_kind_the_user_may_not_list_shows_the_reason(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.f.ports()
        .resources
        .script()
        .watch
        .push_err(OxiError::forbidden("pods is forbidden: User cannot list"));
    let view = d.open(&pod_ref("web-0"));
    match d.read(&view, |v| v.state().clone()) {
        DetailState::Unavailable(message) => assert!(message.contains("forbidden"), "{message}"),
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert!(d.shown("detail-banner"));
}

#[gpui::test]
fn closing_the_drawer_releases_the_feeds(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let view = d.open(&pod_ref("web-0"));
    assert!(d.f.ports().resources.live_watches() >= 1);
    d.click("detail-close");
    assert!(d.drawer_view().is_none(), "the drawer dropped the detail");
    let workspace = d.workspace();
    let open = d.f.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .dock(oxikube_workspace::DockPosition::Right, cx)
            .map(|dock| dock.is_open())
    });
    assert_eq!(open, Some(false), "the dock closed");
    // The store keeps an unused feed for its grace period, then drops it. The test still holds
    // the view's handle, so only `release` (not the view's drop) can have let go of the feed.
    d.f.clock.advance(Duration::from_secs(60));
    d.settle();
    assert_eq!(
        d.f.ports().resources.live_watches(),
        0,
        "closing the drawer stops the object's watch"
    );
    drop(view);
}
