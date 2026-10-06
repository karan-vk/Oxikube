//! Connection states shown on the rows come from the session manager, whoever changes them.

use std::cell::Cell;
use std::rc::Rc;

use gpui::TestAppContext;

use super::{Fixture, Setup, contexts, id};
use crate::catalog::test_support::context;

fn connect(f: &mut Fixture, name: &str) {
    let manager = f.sessions.clone();
    let cluster = id(name);
    let task = f
        .app
        .cx()
        .executor()
        .spawn(async move { manager.connect(&cluster).await });
    f.app.run_until_parked();
    drop(task);
}

#[gpui::test]
fn a_session_changed_by_something_else_updates_the_badge(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    connect(&mut f, "ctx-02");
    let badge = f.read(|v| v.model().badge(2)).unwrap();
    assert_eq!(badge.label, "Connected");

    f.sessions.disconnect(&id("ctx-02")).unwrap();
    f.app.run_until_parked();
    assert_eq!(
        f.read(|v| v.model().badge(2)).unwrap().label,
        "Disconnected"
    );
}

#[gpui::test]
fn a_session_that_was_already_connected_when_the_view_opened_is_shown(cx: &mut TestAppContext) {
    let mut f = Fixture::start(cx, Setup::new(contexts(2)).connected(&["ctx-01"]));
    assert_eq!(
        f.read(|v| v.model().badge(0)).unwrap().label,
        "Disconnected"
    );
    assert_eq!(f.read(|v| v.model().badge(1)).unwrap().label, "Connected");
}

#[gpui::test]
fn sessions_of_clusters_not_in_the_catalog_do_not_redraw_the_catalog(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(2));
    let redraws = Rc::new(Cell::new(0));
    let view = f.view.clone();
    let counter = redraws.clone();
    let _watch = f
        .app
        .update(move |cx| cx.observe(&view, move |_, _| counter.set(counter.get() + 1)));
    f.app.run_until_parked();
    let before = redraws.get();

    f.sessions.open(&context("elsewhere"), Default::default());
    connect(&mut f, "elsewhere");
    f.advance_a_frame();
    assert_eq!(redraws.get(), before, "nothing on screen changed");

    connect(&mut f, "ctx-01");
    f.advance_a_frame();
    assert!(redraws.get() > before, "a listed cluster does redraw");
}

#[gpui::test]
fn a_burst_of_updates_redraws_once(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(20));
    let redraws = Rc::new(Cell::new(0));
    let view = f.view.clone();
    let counter = redraws.clone();
    let _watch = f
        .app
        .update(move |cx| cx.observe(&view, move |_, _| counter.set(counter.get() + 1)));
    f.app.run_until_parked();
    let before = redraws.get();
    for ix in 0..20 {
        futures::executor::block_on(f.sessions.connect(&id(&format!("ctx-{ix:02}"))))
            .expect("connect");
    }
    f.app.run_until_parked();
    f.advance_a_frame();
    assert_eq!(redraws.get() - before, 1, "20 sessions opened, one redraw");
    assert_eq!(f.read(|v| v.model().badge(19)).unwrap().label, "Connected");
}

#[gpui::test]
fn a_lagged_update_stream_forgets_the_sessions_that_closed_meanwhile(cx: &mut TestAppContext) {
    let mut f = Fixture::start(
        cx,
        Setup::new(contexts(6))
            .connected(&["ctx-00"])
            .update_capacity(2),
    );
    assert_eq!(f.read(|v| v.model().badge(0)).unwrap().label, "Connected");

    // The view does not run between these calls: its stream overflows and loses the
    // `Closed` of ctx-00.
    assert!(f.sessions.close(&id("ctx-00")));
    for ix in 1..6 {
        futures::executor::block_on(f.sessions.connect(&id(&format!("ctx-{ix:02}"))))
            .expect("connect");
    }
    f.app.run_until_parked();

    assert_eq!(
        f.read(|v| v.model().badge(0)).unwrap().label,
        "Disconnected",
        "no session, no Connected badge"
    );
    assert_eq!(f.read(|v| v.model().badge(5)).unwrap().label, "Connected");
}
