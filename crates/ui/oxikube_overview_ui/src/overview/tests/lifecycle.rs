//! When the overview runs: its two feeds are held exactly while it is displayed, its cluster's tab
//! shown and the cluster connected; hiding, closing or disconnecting releases them.

use gpui::TestAppContext;
use oxikube_testkit::{node, pod};
use oxikube_workspace::Item as _;

use super::{Fixture, Setup};

fn seeded() -> Setup {
    Setup {
        objects: vec![
            node().name("n1").build(),
            pod().namespace("a").name("p").running().node("n1").build(),
        ],
        ..Setup::default()
    }
}

#[gpui::test]
fn the_overview_holds_one_pod_and_one_node_feed_while_displayed(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, seeded());
    f.tick();
    assert!(f.read(|o| o.is_live()));
    assert_eq!(
        f.ports.resources.live_watches(),
        2,
        "the Pod and Node feeds"
    );

    let overview = f.overview.clone();
    f.vcx.update(|window, cx| {
        overview.update(cx, |o, cx| o.set_active(false, window, cx));
    });
    f.tick();
    assert!(!f.read(|o| o.is_live()));
    assert_eq!(
        f.ports.resources.live_watches(),
        0,
        "a hidden overview releases its feeds"
    );
    assert_eq!(
        f.read(|o| o.shown().snapshot.phases.running),
        1,
        "the last numbers stay"
    );

    f.vcx.update(|window, cx| {
        overview.update(cx, |o, cx| o.set_active(true, window, cx));
    });
    f.tick();
    assert_eq!(
        f.ports.resources.live_watches(),
        2,
        "shown again, it follows again"
    );
}

#[gpui::test]
fn a_hidden_cluster_tab_releases_the_feeds(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, seeded());
    f.tick();
    let overview = f.overview.clone();
    f.vcx
        .update(|_, cx| overview.update(cx, |o, cx| o.set_tab_shown(false, cx)));
    f.tick();
    assert_eq!(f.ports.resources.live_watches(), 0);
    f.vcx
        .update(|_, cx| overview.update(cx, |o, cx| o.set_tab_shown(true, cx)));
    f.tick();
    assert_eq!(f.ports.resources.live_watches(), 2);
}

#[gpui::test]
fn an_overview_opened_in_a_hidden_cluster_tab_holds_nothing_until_shown(cx: &mut TestAppContext) {
    // A cluster that connects while another tab is shown gets its first screen in the background.
    let mut f = Fixture::open(
        cx,
        Setup {
            tab_shown: false,
            ..seeded()
        },
    );
    f.tick();
    assert!(!f.read(|o| o.is_live()));
    assert_eq!(
        f.ports.resources.live_watches(),
        0,
        "no feeds for a tab nobody sees"
    );
    let overview = f.overview.clone();
    f.vcx
        .update(|_, cx| overview.update(cx, |o, cx| o.set_tab_shown(true, cx)));
    f.tick();
    assert!(f.read(|o| o.is_live()));
    assert_eq!(f.ports.resources.live_watches(), 2);
}

#[gpui::test]
fn closing_the_overview_releases_the_feeds(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, seeded());
    f.tick();
    let (ws, id) = (f.ws.clone(), f.overview.entity_id());
    f.vcx
        .update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(id, window, cx)));
    f.tick();
    assert_eq!(f.ports.resources.live_watches(), 0);
}

#[gpui::test]
fn a_disconnected_cluster_says_so_and_holds_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::open(
        cx,
        Setup {
            connect: false,
            ..seeded()
        },
    );
    f.tick();
    assert!(!f.read(|o| o.is_live()));
    assert!(f.drawn("overview-not-connected"));
    assert_eq!(f.ports.resources.live_watches(), 0);

    futures::executor::block_on(f.sessions.connect(&super::cluster())).expect("connect");
    f.tick();
    assert!(
        f.read(|o| o.is_live()),
        "it starts when the cluster connects"
    );
    assert!(f.drawn("overview-capacity"));

    f.sessions
        .disconnect(&super::cluster())
        .expect("disconnect");
    f.tick();
    assert!(!f.read(|o| o.is_live()));
    assert_eq!(f.ports.resources.live_watches(), 0);
}
