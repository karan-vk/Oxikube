//! The Events tab over the app's events index (E14-S04): every kind has it, it lists the object's
//! events newest first from the index, opens no watch of its own, follows new events, and says
//! when the events are unavailable.

use gpui::TestAppContext;
use jiff::SignedDuration;
use oxikube_domain::command::Command;
use oxikube_domain::event::EventType;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{OxiError, Resource};
use oxikube_testkit::{FakeEventFeed, ResourceCall, event, node};
use serde_json::json;

use super::support::warning_event;
use crate::detail::tests::fixture::{Detail, edited, pod_ref, web_pod};
use crate::detail::{DetailTab, DetailView};
use crate::table::tests::fixture::cluster;

const MIN: SignedDuration = SignedDuration::from_mins(1);

fn pod_with_uid() -> Resource {
    edited(web_pod(), |json| {
        json["metadata"]["uid"] = json!("uid-web-0");
    })
}

/// A `Normal` event about `shop/web-0`, last seen `age` ago.
fn normal_about_web(uid: &str, reason: &str, age: SignedDuration) -> oxikube_domain::event::Event {
    let mut e = event()
        .uid(uid)
        .reason(reason)
        .message(&format!("{reason} web-0"))
        .regarding("Pod", Some("shop"), "web-0", "uid-web-0")
        .build();
    e.last_seen = Some(jiff::Timestamp::now() - age);
    e.first_seen = e.last_seen;
    e
}

fn reasons(d: &mut Detail, view: &gpui::Entity<DetailView>) -> Vec<String> {
    d.read(view, |v| {
        v.event_rows()
            .iter()
            .map(|e| e.reason.to_string())
            .collect()
    })
}

#[gpui::test]
fn the_tab_lists_the_objects_events_newest_first_from_the_index(cx: &mut TestAppContext) {
    let feed = FakeEventFeed::new().initial([
        normal_about_web("e1", "Pulled", 30 * MIN),
        warning_event("e2", "shop", "web-0", "uid-web-0", "BackOff", 2 * MIN),
        normal_about_web("e3", "Started", 10 * MIN),
        // About another pod of the namespace.
        warning_event("e4", "shop", "web-1", "uid-web-1", "Failed", MIN),
    ]);
    let (mut d, rig) = Detail::with_events(cx, [pod_with_uid()], feed);
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-events");
    d.settle();

    assert_eq!(reasons(&mut d, &view), ["BackOff", "Started", "Pulled"]);
    let rows = d.read(&view, |v| v.event_rows().to_vec());
    assert_eq!(rows[0].kind, EventType::Warning);
    assert_eq!(rows[0].type_label(), "Warning");
    assert_eq!(rows[0].message, "BackOff for web-0");
    assert_eq!(rows[1].type_label(), "Normal");
    assert!(d.shown("detail-event-0") && d.shown("detail-event-2"));
    assert!(!d.shown("detail-event-3"));
    assert!(
        d.shown("detail-event-type-0"),
        "the type is drawn as a word"
    );

    // No second watch: the store never watched Events, and the service has the one feed.
    let store_event_watches = d
        .f
        .ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, ResourceCall::Watch { kind, .. } if &*kind.kind == "Event"))
        .count();
    assert_eq!(store_event_watches, 0, "the tab reads the index");
    assert_eq!(rig.port.live_feeds(), 1);
    assert_eq!(rig.port.recorded_calls().len(), 1);

    // Back and forth opens nothing more.
    d.click("detail-tab-overview");
    d.click("detail-tab-events");
    assert_eq!(rig.port.recorded_calls().len(), 1);
    assert_eq!(reasons(&mut d, &view).len(), 3);
}

#[gpui::test]
fn a_new_event_arrives_at_the_top(cx: &mut TestAppContext) {
    let feed = FakeEventFeed::new()
        .initial([normal_about_web("e1", "Pulled", 30 * MIN)])
        .add(
            1,
            warning_event(
                "e2",
                "shop",
                "web-0",
                "uid-web-0",
                "Unhealthy",
                SignedDuration::from_secs(1),
            ),
        );
    let (mut d, rig) = Detail::with_events(cx, [pod_with_uid()], feed);
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-events");
    d.settle();
    assert_eq!(reasons(&mut d, &view), ["Pulled"]);
    rig.tick();
    d.settle();
    assert_eq!(reasons(&mut d, &view), ["Unhealthy", "Pulled"]);
}

#[gpui::test]
fn an_object_without_events_says_so(cx: &mut TestAppContext) {
    let (mut d, _rig) = Detail::with_events(cx, [pod_with_uid()], FakeEventFeed::new());
    d.open(&pod_ref("web-0"));
    d.click("detail-tab-events");
    d.settle();
    assert!(d.shown("detail-events-empty"));
}

#[gpui::test]
fn a_cluster_that_refuses_events_says_they_are_unavailable(cx: &mut TestAppContext) {
    let mut f = crate::table::tests::fixture::Fixture::new(cx);
    f.ports()
        .events
        .script()
        .watch_events
        .push_err(OxiError::forbidden("events are forbidden"));
    let _rig = f.install_events(FakeEventFeed::new());
    f.ports()
        .discovery
        .set_kinds(crate::detail::tests::fixture::kinds());
    f.ports().resources.insert(pod_with_uid());
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    let mut d = Detail { f };
    d.open(&pod_ref("web-0"));
    d.click("detail-tab-events");
    d.settle();
    assert!(d.shown("detail-events-error"));
}

#[gpui::test]
fn every_kind_has_the_tab_and_a_node_is_found_by_its_name(cx: &mut TestAppContext) {
    // A Deployment, by uid.
    let deployment = edited(
        oxikube_testkit::deployment()
            .namespace("shop")
            .name("web")
            .build(),
        |json| json["metadata"]["uid"] = json!("uid-deploy"),
    );
    // A Node: the kubelet's events carry the node's name as the uid.
    let mut kubelet = warning_event(
        "n1",
        "default",
        "kind-cp",
        "kind-cp",
        "NodeNotReady",
        3 * MIN,
    );
    kubelet.regarding.gvk = Gvk::new("", "v1", "Node");
    kubelet.regarding.namespace = None;
    let mut by_deploy = warning_event("d1", "shop", "web", "uid-deploy", "FailedCreate", 2 * MIN);
    by_deploy.regarding.gvk = Gvk::new("apps", "v1", "Deployment");
    let feed = FakeEventFeed::new().initial([kubelet, by_deploy]);
    let cp = edited(node().name("kind-cp").build(), |json| {
        json["metadata"]["uid"] = json!("real-node-uid");
    });
    let (mut d, _rig) = Detail::with_events(cx, [pod_with_uid(), deployment, cp], feed);

    for (target, expected) in [
        (pod_ref("web-0"), Vec::<&str>::new()),
        (
            ResourceRef::namespaced(
                cluster(),
                Gvk::new("apps", "v1", "Deployment"),
                "shop",
                "web",
            ),
            vec!["FailedCreate"],
        ),
        (
            ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "kind-cp"),
            vec!["NodeNotReady"],
        ),
    ] {
        let view = d.open(&target);
        assert!(
            DetailTab::for_kind(&target.gvk).contains(&DetailTab::Events),
            "{} has the Events tab",
            target.gvk.kind
        );
        assert!(d.shown("detail-tab-events"));
        d.click("detail-tab-events");
        d.settle();
        assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Events);
        assert_eq!(reasons(&mut d, &view), expected, "{}", target.gvk.kind);
    }
}

#[gpui::test]
fn view_events_opens_the_drawer_on_the_events_tab(cx: &mut TestAppContext) {
    let feed = FakeEventFeed::new().initial([normal_about_web("e1", "Pulled", MIN)]);
    let (mut d, _rig) = Detail::with_events(cx, [pod_with_uid()], feed);
    let target = pod_ref("web-0");
    // What the badge click dispatches, through the recording dispatcher and the views' queue.
    d.f.vcx.update(|_, cx| {
        use oxikube_workspace::CommandDispatcher as _;
        d.f.dispatcher.dispatch(
            Command::ResourceViewEvents {
                target: target.clone(),
            },
            cx,
        );
    });
    d.settle();
    let view = d.drawer_view().expect("the drawer opened");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Events);
    assert_eq!(reasons(&mut d, &view), ["Pulled"]);
}
