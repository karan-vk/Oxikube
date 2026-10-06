//! Session restore (E06-S11): the orderings a plain run does not reach. Closing a placeholder
//! the restore has queued, and the restore waiting for the layout restore (E05-S05).

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_testkit::FakeStatePort;

use super::restore::Restore;
use super::*;
use crate::cluster_tab::RestoreConnectSetting;
use crate::cluster_tab::tab::status_text;
use crate::persistence::tests::{gated::GatedState, store};
use crate::persistence::{LayoutPersistence, RestoreStatus};

#[gpui::test]
fn closing_a_queued_placeholder_keeps_the_restore_from_connecting_it(cx: &mut TestAppContext) {
    // Concurrency is 2: a and b connect first (and hang), c and d queue.
    let mut r = Restore::new(
        cx,
        &["a", "b", "c", "d"],
        &["a", "b", "c", "d"],
        Some("a"),
        true,
        RestoreConnectSetting::All,
    );
    r.fx.connector.hold();
    r.start_and_draw();
    assert_eq!(r.connects(), ["a", "b"], "c and d wait for a slot");
    assert!(r.placeholder("c") && r.placeholder("d"));

    // The user dismisses c while it is still queued.
    assert!(r.fx.apply(Command::ClusterCloseTab { cluster: id("c") }));
    assert_eq!(r.fx.open_names(), ["a", "b", "d"]);

    // A slot frees up: d is next, c is not.
    r.fx.connector.release();
    r.fx.vcx.run_until_parked();

    assert_eq!(r.connects(), ["a", "b", "d"], "c was never connected");
    assert_eq!(
        r.fx.open_names(),
        ["a", "b", "d"],
        "and the tab the user closed did not come back"
    );
    assert_eq!(
        r.fx.sessions.get(&id("c")).map(|s| s.phase()),
        Some(SessionPhase::Disconnected)
    );
    assert_eq!(r.state_of("d"), ClusterSessionState::Ready);
}

#[gpui::test]
fn closing_a_placeholder_whose_connect_just_started_disconnects_it(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    r.fx.connector.hold();
    r.start_and_draw();
    assert!(r.placeholder("b"));

    // b's connect starts (the user's own, from elsewhere) but the tabs have not heard yet: the
    // tab is still a placeholder, the session no longer disconnected.
    let sessions = r.fx.sessions.clone();
    let mut in_flight = Box::pin(async move { sessions.connect(&id("b")).await });
    // Hangs on the held connector: polled once to move the session to Connecting.
    assert!(!futures::executor::block_on(async {
        futures::poll!(in_flight.as_mut()).is_ready()
    }));
    assert_eq!(
        r.fx.sessions.get(&id("b")).map(|s| s.phase()),
        Some(SessionPhase::Connecting)
    );
    assert!(r.placeholder("b"), "the tabs have not heard about it yet");

    r.fx.apply(Command::ClusterCloseTab { cluster: id("b") });

    assert_eq!(
        r.fx.recorder.disconnects(),
        1,
        "an ordinary close: the connect is cancelled, not left to reopen the tab"
    );
    assert_eq!(
        r.fx.sessions.get(&id("b")).map(|s| s.phase()),
        Some(SessionPhase::Disconnected)
    );
    assert_eq!(r.fx.open_names(), ["a"]);
}

/// The layout restore of the main window, held open until the returned gate is opened.
fn held_layout(
    r: &mut Restore,
) -> (
    gpui::Entity<LayoutPersistence>,
    futures::channel::oneshot::Sender<()>,
) {
    let (gated, open) = GatedState::new(Arc::new(FakeStatePort::new()));
    let ws = r.fx.ws.clone();
    let layout =
        r.fx.vcx
            .update(|window, cx| LayoutPersistence::start(&ws, store(gated), window, cx));
    (layout, open)
}

#[gpui::test]
fn the_restore_waits_for_the_layout_restore(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    let (layout, open) = held_layout(&mut r);
    r.fx.vcx.run_until_parked();
    let restoring =
        r.fx.vcx
            .update(|_, cx| *layout.read(cx).status() == RestoreStatus::Restoring);
    assert!(restoring, "the layout is still loading");

    r.start_after(Some(layout.clone()));
    r.first_frame();

    // The first frame is on screen, but the layout has not come back: nothing opens or connects.
    assert!(r.fx.open_names().is_empty(), "no placeholder yet");
    assert!(r.fx.sessions.sessions().is_empty());
    assert!(r.connects().is_empty());

    open.send(()).expect("the layout restore is waiting");
    r.fx.vcx.run_until_parked();

    assert_ne!(
        r.fx.vcx.update(|_, cx| layout.read(cx).status().clone()),
        RestoreStatus::Restoring
    );
    assert_eq!(r.fx.open_names(), ["a", "b"], "the layout is back: restore");
    assert_eq!(r.connects(), ["a"]);
}

#[gpui::test]
fn a_finished_layout_restore_does_not_hold_the_restore_up(cx: &mut TestAppContext) {
    let mut r = Restore::new(
        cx,
        &["a", "b"],
        &["a", "b"],
        Some("a"),
        true,
        RestoreConnectSetting::Active,
    );
    let ws = r.fx.ws.clone();
    let layout = r.fx.vcx.update(|window, cx| {
        LayoutPersistence::start(&ws, store(Arc::new(FakeStatePort::new())), window, cx)
    });
    r.fx.vcx.run_until_parked();

    r.start_after(Some(layout));
    r.first_frame();

    assert_eq!(r.fx.open_names(), ["a", "b"]);
}

#[test]
fn the_placeholder_says_what_the_session_is_doing() {
    assert_eq!(status_text(&ClusterSessionState::Connecting), "Connecting…");
    assert_eq!(
        status_text(&ClusterSessionState::Disconnected),
        "Disconnected"
    );
    let failed = status_text(&ClusterSessionState::Error {
        reason: "the handshake was refused".into(),
    });
    assert_eq!(failed, "Connection failed: the handshake was refused");
    let auth = status_text(&ClusterSessionState::AuthRequired {
        reason: "token expired".into(),
    });
    assert!(auth.contains("token expired"), "{auth}");
}
