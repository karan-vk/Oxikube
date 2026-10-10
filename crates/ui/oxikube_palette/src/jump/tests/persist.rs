//! The lines run in the bar are kept between runs (E11-S11): recorded per cluster through the
//! state store, and read back when the bar opens so `[` reaches the last run's lines.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_ports::{StateKey, StatePort};
use serde_json::json;

use super::{Fixture, id, wait_for_data};
use crate::jump::JumpRequest;

fn key(name: &str) -> StateKey {
    StateKey::new(format!("history.jump/{}", id(name))).expect("a state key")
}

#[gpui::test]
fn a_confirmed_line_is_recorded_for_the_shown_cluster(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("deploy /api web");
    f.run_line("pods");
    assert_eq!(
        f.recents.recent(&id("dev")),
        ["pods", "deploy web /api"],
        "latest first, the canonical text"
    );
    assert!(f.recents.recent(&id("prod")).is_empty());
}

#[gpui::test]
fn lines_that_are_not_navigation_are_not_recorded(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("q");
    f.run_line("-");
    assert!(f.recents.recent(&id("dev")).is_empty());
}

#[gpui::test]
fn the_lines_of_the_last_run_come_back_when_the_bar_opens(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    block_on(f.state.kv_set(
        &key("dev"),
        json!({ "v": 1, "jumps": ["ns", "deploy web", "pods"] }),
    ))
    .expect("stored");
    f.open();
    wait_for_data(&mut f);
    f.keys("escape");
    assert_eq!(
        f.host.history().lines(),
        ["pods", "deploy web", "ns"],
        "oldest first, the way the ring runs"
    );
    let host = f.host.clone();
    f.vcx.update(|window, cx| {
        host.apply(
            JumpRequest::Step(oxikube_app::search::jump::HistoryStep::Back),
            window,
            cx,
        )
    });
    let sent = f.take_sent();
    assert!(!sent.is_empty(), "`[` runs the line before `ns`: {sent:?}");
}

#[gpui::test]
fn a_ring_that_has_lines_is_not_reordered_by_the_stored_ones(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("pods");
    block_on(
        f.state
            .kv_set(&key("dev"), json!({ "v": 1, "jumps": ["ns"] })),
    )
    .expect("stored");
    f.open();
    wait_for_data(&mut f);
    assert_eq!(f.host.history().lines(), ["pods"]);
}
