//! Events in the running app (E14-S02): a connected cluster's tab subscribes to its warnings, so
//! the app's `EventService` opens the cluster's one events feed (over the fake port), and the
//! cluster overview's "Recent warnings" panel shows them, newest first. A disconnect stops the
//! feed and empties the panel's source.

use gpui::TestAppContext;
use oxikube_app::EventFilter;
use oxikube_overview_ui::ClusterOverview;
use oxikube_testkit::{FakeEventFeed, TICK, TestPorts, event};

use super::App;
use crate::app_state::AppState;

#[gpui::test]
fn a_connected_clusters_warnings_reach_the_overview_from_one_feed(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let events = ports.connector.ports_for(&cluster).events;
    events.install(
        FakeEventFeed::new()
            .initial([
                event().uid("n").reason("Pulled").build(),
                event()
                    .uid("w")
                    .warning()
                    .reason("BackOff")
                    .message("Back-off restarting failed container")
                    .regarding("Pod", Some("default"), "web-0", "pod-uid")
                    .count(3)
                    .build(),
            ])
            .add(
                1,
                event()
                    .uid("w2")
                    .warning()
                    .reason("FailedScheduling")
                    .regarding("Pod", Some("default"), "web-1", "pod-uid-1")
                    .last_seen("2026-01-01T00:05:00Z")
                    .build(),
            ),
    );
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();

    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let service = state.services().events.clone();
    assert_eq!(events.live_feeds(), 1, "one events feed for the cluster");
    assert_eq!(events.recorded_calls().len(), 1);
    assert_eq!(service.query(&cluster, &EventFilter::new()).len(), 2);

    let overview = app.tab_workspace();
    let overview = app
        .vcx
        .update(|_, cx| overview.read(cx).items_of_type::<ClusterOverview>()[0].clone());
    let reasons = |app: &mut App| {
        app.vcx.update(|_, cx| {
            overview
                .read(cx)
                .warnings()
                .iter()
                .map(|w| (w.object.clone(), w.reason.clone(), w.count))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        reasons(&mut app),
        [("Pod/web-0".into(), "BackOff".into(), 3)]
    );

    // A new warning arrives on the feed: the panel shows it first.
    events.clock().advance(TICK);
    app.tick();
    assert_eq!(
        reasons(&mut app),
        [
            ("Pod/web-1".into(), "FailedScheduling".into(), 1),
            ("Pod/web-0".into(), "BackOff".into(), 3)
        ]
    );
    assert_eq!(events.live_feeds(), 1, "still one feed");

    // Disconnecting stops the feed and forgets the events.
    state
        .services()
        .sessions
        .disconnect(&cluster)
        .expect("disconnect");
    app.tick();
    assert_eq!(events.live_feeds(), 0);
    assert!(service.query(&cluster, &EventFilter::new()).is_empty());
}
