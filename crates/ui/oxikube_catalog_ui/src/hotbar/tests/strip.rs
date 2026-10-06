//! What the strip shows and what its tiles do.

use gpui::TestAppContext;
use oxikube_domain::ClusterColour;
use oxikube_domain::command::Command;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};

use super::*;

const RED: ClusterColour = ClusterColour::rgb(0xe5, 0x39, 0x35);
const BLUE: ClusterColour = ClusterColour::rgb(0x1e, 0x88, 0xe5);

#[gpui::test]
fn shows_connected_and_favourite_clusters_with_their_colour(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(
        cx,
        &["prod", "staging", "dev", "unused"],
        Dispatch::Record,
        Arc::new(FakeStatePort::new()),
        |p| {
            p.connect_with_colour("prod", Some(RED));
            p.connect_with_colour("staging", Some(BLUE));
            p.favourite("dev");
        },
    );

    // Connected clusters first, then the favourite; the other context is not on the strip.
    assert_eq!(fx.tiles(), ["prod", "staging", "dev"]);
    for name in ["prod", "staging", "dev"] {
        assert!(
            fx.drawn(&format!("hotbar-tile-{name}")),
            "{name} has a tile"
        );
        assert!(
            fx.drawn(&format!("hotbar-colour-{name}")),
            "{name} has a colour dot"
        );
    }
    assert!(!fx.drawn("hotbar-tile-unused"));

    // The colour dots carry the sessions' colours; a cluster with none gets the accent.
    let entries = fx.vcx.update(|_, cx| fx.hotbar.read(cx).model().entries());
    assert_eq!(entries[0].colour, Some(RED));
    assert_eq!(entries[1].colour, Some(BLUE));
    assert_eq!(entries[2].colour, None);
    assert_eq!(entries[0].initials, "PR");
    assert_eq!(entries[1].initials, "ST");

    // Connected ones show what the session is doing; a favourite that is not connected shows
    // nothing and reads as disconnected.
    assert!(fx.drawn("hotbar-state-prod") && fx.drawn("hotbar-state-staging"));
    assert!(!fx.drawn("hotbar-state-dev"));
    assert!(entries[0].connected && !entries[2].connected);
    assert!(entries[2].favourite);
    assert_eq!(entries[2].state, ClusterSessionState::Disconnected);
}

#[gpui::test]
fn the_strip_is_at_the_left_edge_of_the_window(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a"]);
    fx.favourite("a", true);
    let hotbar = fx.bounds("hotbar".into()).expect("the strip is drawn");
    let body = fx.bounds("workspace-body".into());
    assert_eq!(hotbar.origin.x, gpui::px(0.));
    assert!(hotbar.size.width > gpui::px(0.));
    if let Some(body) = body {
        assert!(
            body.origin.x >= hotbar.origin.x + hotbar.size.width,
            "docks start after it"
        );
    }
}

#[gpui::test]
fn connecting_and_disconnecting_adds_and_removes_tiles(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a", "b"]);
    assert!(fx.tiles().is_empty());
    fx.connect("a");
    fx.connect("b");
    assert_eq!(fx.tiles(), ["a", "b"]);
    fx.sessions.disconnect(&id("a")).expect("disconnect");
    fx.vcx.run_until_parked();
    assert_eq!(fx.tiles(), ["b"], "a disconnected non-favourite leaves");
    assert!(!fx.drawn("hotbar-tile-a"));
    fx.sessions.close(&id("b"));
    fx.vcx.run_until_parked();
    assert!(fx.tiles().is_empty());
}

#[gpui::test]
fn a_favourite_stays_when_it_disconnects(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a"]);
    fx.favourite("a", true);
    fx.connect("a");
    assert!(fx.drawn("hotbar-state-a"));
    fx.sessions.disconnect(&id("a")).expect("disconnect");
    fx.vcx.run_until_parked();
    assert_eq!(fx.tiles(), ["a"]);
    assert!(
        !fx.drawn("hotbar-state-a"),
        "no state for a cluster that is only a favourite"
    );
}

#[gpui::test]
fn favourites_marked_in_the_catalog_appear_and_disappear(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a", "b"]);
    assert!(fx.tiles().is_empty());
    // The star in the catalog, the palette and an agent all end in the catalog.
    fx.favourite("b", true);
    assert_eq!(fx.tiles(), ["b"]);
    fx.favourite("a", true);
    assert_eq!(fx.tiles(), ["a", "b"], "favourites by name");
    fx.favourite("b", false);
    assert_eq!(fx.tiles(), ["a"]);
}

#[gpui::test]
fn the_state_and_the_colour_follow_the_session(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a"]);
    fx.connect("a");
    let state = |fx: &mut Fixture| {
        fx.vcx
            .update(|_, cx| {
                fx.hotbar
                    .read(cx)
                    .model()
                    .find(&id("a"))
                    .map(|e| (e.state, e.colour))
            })
            .expect("tile")
    };
    assert_eq!(state(&mut fx), (ClusterSessionState::Ready, None));
    fx.sessions.set_colour(&id("a"), Some(RED)).expect("open");
    fx.vcx.run_until_parked();
    assert_eq!(state(&mut fx).1, Some(RED));
    assert!(
        fx.sessions
            .report_health(&id("a"), oxikube_ports::HealthSignal::Unhealthy)
    );
    fx.vcx.run_until_parked();
    assert_eq!(state(&mut fx).0.phase(), SessionPhase::Degraded);
}

#[gpui::test]
fn clicking_a_connected_cluster_selects_and_a_favourite_connects(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(
        cx,
        &["prod", "dev"],
        Dispatch::Record,
        Arc::new(FakeStatePort::new()),
        |p| {
            p.connect_with_colour("prod", Some(RED));
            p.favourite("dev");
        },
    );
    fx.connect("dev");
    fx.sessions.disconnect(&id("dev")).expect("disconnect");
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("prod"));

    fx.click("hotbar-tile-dev".into());
    // `cluster::Connect` went to the services (here: the recorder); the favourite has no tab.
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterConnect { cluster: id("dev") }]
    );
    fx.recorder.clear();

    // `cluster::Select` is a tab command: the tabs ran it, the services never saw it.
    fx.connect("dev");
    assert_eq!(fx.active_name().as_deref(), Some("dev"));
    fx.click("hotbar-tile-prod".into());
    assert_eq!(fx.active_name().as_deref(), Some("prod"));
    assert!(fx.recorder.sent().is_empty(), "{:?}", fx.recorder.sent());
}

#[gpui::test]
fn clicking_a_tile_shows_that_clusters_tab(cx: &mut TestAppContext) {
    let mut fx = Fixture::run(cx, &["a", "b", "c"]);
    for name in ["a", "b", "c"] {
        fx.connect(name);
    }
    assert_eq!(fx.active_name().as_deref(), Some("c"));
    assert!(fx.drawn("hotbar-active-c") && !fx.drawn("hotbar-active-a"));

    fx.click("hotbar-tile-a".into());
    assert_eq!(fx.active_name().as_deref(), Some("a"));
    assert!(fx.drawn("hotbar-active-a"), "the bar moved to a");
    assert!(!fx.drawn("hotbar-active-c"));
}

#[gpui::test]
fn clicking_a_favourite_connects_it_and_its_tab_opens_displayed(cx: &mut TestAppContext) {
    let mut fx = Fixture::run(cx, &["a", "b"]);
    fx.favourite("b", true);
    assert!(fx.sessions.get(&id("b")).is_none());

    fx.click("hotbar-tile-b".into());

    let phase = fx.sessions.get(&id("b")).map(|s| s.phase());
    assert_eq!(phase, Some(SessionPhase::Ready));
    assert_eq!(fx.active_name().as_deref(), Some("b"));
    assert!(fx.drawn("hotbar-state-b") && fx.drawn("hotbar-active-b"));
}

#[gpui::test]
fn the_catalog_catches_up_with_a_favourite_toggled_on_the_hotbar(cx: &mut TestAppContext) {
    let mut fx = Fixture::run(cx, &["a"]);
    fx.connect("a");
    let hotbar = fx.hotbar.clone();
    let toggle = |fx: &mut Fixture, favourite: bool| {
        fx.vcx.update(|_, cx| {
            hotbar.update(cx, |hotbar, cx| {
                hotbar.toggle_favourite(&id("a"), "a", favourite, cx)
            })
        });
        fx.vcx.run_until_parked();
    };
    let favourite = |fx: &mut Fixture| {
        block_on(fx.catalog.load())
            .expect("load")
            .iter()
            .any(|e| e.name() == "a" && e.favourite)
    };
    toggle(&mut fx, true);
    assert!(favourite(&mut fx), "stored through the catalog");
    toggle(&mut fx, false);
    assert!(!favourite(&mut fx));
    assert_eq!(fx.tiles(), ["a"], "still connected, so still there");
}

#[gpui::test]
fn the_favourite_toggle_is_the_cluster_command(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["a"]);
    fx.connect("a");
    let hotbar = fx.hotbar.clone();
    fx.vcx.update(|_, cx| {
        hotbar.update(cx, |hotbar, cx| {
            hotbar.toggle_favourite(&id("a"), "a", true, cx)
        })
    });
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterToggleFavourite {
            cluster: id("a"),
            favourite: Some(true)
        }]
    );
    // The tile shows it at once.
    let favourite = fx.vcx.update(|_, cx| {
        fx.hotbar
            .read(cx)
            .model()
            .find(&id("a"))
            .map(|e| e.favourite)
    });
    assert_eq!(favourite, Some(true));
}
