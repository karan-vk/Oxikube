//! Connecting: Enter and click dispatch `cluster::Connect`; the star dispatches the favourite
//! command; and wired to the real handler the connection happens and the badge follows.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::session::SessionPhase;
use oxikube_ports::ClockPort as _;

use super::{Fixture, Setup, contexts, id};
use crate::catalog::Tone;

#[gpui::test]
fn clicking_a_row_dispatches_connect_for_that_cluster(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(5));
    f.window.draw_frame();
    f.click("catalog-row-3");
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterConnect {
            cluster: id("ctx-03")
        }]
    );
    assert_eq!(
        f.read(|v| v.model().selected_index()),
        Some(3),
        "the clicked row is selected"
    );
}

#[gpui::test]
fn clicking_the_star_toggles_the_favourite_and_does_not_connect(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(5));
    f.window.draw_frame();
    f.click("catalog-star-2");
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterToggleFavourite {
            cluster: id("ctx-02"),
            favourite: Some(true)
        }],
        "the click on the star must not also connect the row under it"
    );
    assert_eq!(f.names()[0], "ctx-02");
    f.window.draw_frame();
    f.click("catalog-star-0");
    assert_eq!(f.recorder.sent().len(), 2);
    assert_eq!(
        f.recorder.sent()[1],
        Command::ClusterToggleFavourite {
            cluster: id("ctx-02"),
            favourite: Some(false)
        },
        "the starred cluster is first, so the star in row 0 is its star"
    );
}

#[gpui::test]
fn connecting_shows_the_time_at_once_without_moving_the_row(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    f.window.draw_frame();
    let now = f.clock.now();
    f.click("catalog-row-2");
    assert_eq!(
        f.names(),
        ["ctx-00", "ctx-01", "ctx-02"],
        "no jump under the pointer"
    );
    assert_eq!(
        f.read(|v| v.model().row(2).and_then(|r| r.entry().last_used)),
        Some(now)
    );
}

#[gpui::test]
fn a_click_connects_for_real_and_the_badge_follows_the_session(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(3)).run());
    f.window.draw_frame();
    assert_eq!(
        f.read(|v| v.model().badge(1)).unwrap().label,
        "Disconnected"
    );

    f.click("catalog-row-1");

    let session = f
        .sessions
        .get(&id("ctx-01"))
        .expect("the connect opened a session");
    assert_eq!(session.phase(), SessionPhase::Ready);
    let badge = f.read(|v| v.model().badge(1)).unwrap();
    assert_eq!((badge.label, badge.tone), ("Connected", Tone::Success));
    assert_eq!(
        f.read(|v| v.model().badge(0)).unwrap().label,
        "Disconnected",
        "only that row"
    );
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-status-1"));
}

#[gpui::test]
fn the_connect_is_remembered_in_the_state_db(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(3)).run());
    f.window.draw_frame();
    f.click("catalog-row-1");
    let stored = futures::executor::block_on(f.catalog.load()).unwrap();
    let entry = stored.iter().find(|e| e.name() == "ctx-01").unwrap();
    assert_eq!(entry.last_used, Some(f.clock.now()));
}

#[gpui::test]
fn a_star_is_remembered_across_a_reopen(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(3)).run());
    f.window.draw_frame();
    f.click("catalog-star-2");
    // A reload (the sources changed) shows what was stored, not what was assumed.
    f.update(|view, cx| view.reload(cx));
    assert_eq!(f.names()[0], "ctx-02");
}

#[gpui::test]
fn a_failing_connection_shows_the_error_badge_with_its_reason(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(2)).run());
    f.app.run_until_parked();
    f.connector
        .connect_script_for(&id("ctx-00"))
        .push_err(oxikube_domain::OxiError::internal("connection refused"));
    f.window.draw_frame();
    f.click("catalog-row-0");
    let badge = f.read(|v| v.model().badge(0)).unwrap();
    assert_eq!((badge.label, badge.tone), ("Error", Tone::Error));
    assert!(
        badge
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("refused")),
        "{badge:?}"
    );
}

#[gpui::test]
fn auth_required_has_its_own_badge(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(2)).run());
    f.app.run_until_parked();
    f.connector
        .connect_script_for(&id("ctx-00"))
        .push_err(oxikube_domain::OxiError::auth("token expired", false));
    f.window.draw_frame();
    f.click("catalog-row-0");
    let badge = f.read(|v| v.model().badge(0)).unwrap();
    assert_eq!((badge.label, badge.tone), ("Auth required", Tone::Warning));
}
