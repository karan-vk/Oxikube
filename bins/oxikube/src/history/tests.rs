//! The recents and the jump history reach the state db, and come back from it.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::search::recents::DEBOUNCE;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{StateKey, StatePort};
use oxikube_testkit::TestPorts;
use oxikube_testkit::fakes::StateCall;
use serde_json::json;

use crate::app_state::AppState;

fn written(ports: &TestPorts, key: &str) -> Vec<serde_json::Value> {
    ports
        .state
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            StateCall::KvSet(k, v) if k.as_str() == key => Some(v),
            _ => None,
        })
        .collect()
}

fn seed(ports: &TestPorts, key: &str, value: serde_json::Value) {
    futures::executor::block_on(ports.state.kv_set(&StateKey::new(key).unwrap(), value)).unwrap();
}

#[gpui::test]
fn the_stored_recents_are_there_after_start(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    seed(
        &ports,
        "recents.commands",
        json!({ "v": 1, "ids": ["view::ZoomOut", "pod::Delete"] }),
    );
    let recents = cx.update(|cx| AppState::test_with(cx, &ports).recents());
    cx.run_until_parked();
    assert_eq!(
        recents.recent(),
        [CommandId::VIEW_ZOOM_OUT, CommandId::POD_DELETE]
    );
}

#[gpui::test]
fn a_burst_of_commands_is_written_once_after_the_pause(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    let recents = cx.update(|cx| AppState::test_with(cx, &ports).recents());
    cx.run_until_parked();

    recents.record(CommandId::POD_DELETE);
    recents.record(CommandId::VIEW_ZOOM_IN);
    recents.record(CommandId::VIEW_ZOOM_OUT);
    cx.run_until_parked();
    assert!(
        written(&ports, "recents.commands").is_empty(),
        "inside the pause"
    );

    cx.executor()
        .advance_clock(DEBOUNCE + Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        written(&ports, "recents.commands"),
        [json!({ "v": 1, "ids": ["view::ZoomOut", "view::ZoomIn", "pod::Delete"] })]
    );
}

#[gpui::test]
fn the_jump_history_is_written_per_cluster(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    let jump = cx.update(|cx| AppState::test_with(cx, &ports).jump_history().clone());
    cx.run_until_parked();
    let prod = ClusterId::new("/kubeconfig", &ContextName::new("prod"));
    assert!(jump.record(&prod, "deploy kube-system"));
    cx.executor()
        .advance_clock(DEBOUNCE + Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        written(&ports, &format!("history.jump/{prod}")),
        [json!({ "v": 1, "jumps": ["deploy kube-system"] })]
    );
}

#[gpui::test]
fn what_ran_just_before_quitting_is_flushed(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    let recents = cx.update(|cx| AppState::test_with(cx, &ports).recents());
    cx.run_until_parked();
    recents.record(CommandId::POD_DELETE);
    // The quit comes before the pause is over.
    cx.update(|cx| cx.shutdown());
    assert_eq!(
        written(&ports, "recents.commands"),
        [json!({ "v": 1, "ids": ["pod::Delete"] })]
    );
}

#[gpui::test]
fn start_is_idempotent(cx: &mut TestAppContext) {
    let ports = TestPorts::empty();
    cx.update(|cx| {
        AppState::test_with(cx, &ports);
        super::start(cx);
        super::start(cx);
    });
    cx.run_until_parked();
    let reads = ports
        .state
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, StateCall::KvGet(_)))
        .count();
    assert_eq!(reads, 1, "the recents are read once");
}
