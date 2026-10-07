//! The container picker: opened for a pod with several containers, the default preselected, keys
//! and clicks open, Escape sends nothing.

use gpui::TestAppContext;
use oxikube_app::exec::DEFAULT_CONTAINER_ANNOTATION;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use serde_json::json;

use super::{picker, pod_with, sent_exec, web_ref};
use crate::table::tests::fixture::Fixture;

/// Opens the pods table over `pod` and presses `s` on its row.
fn press_shell(f: &mut Fixture, pod: Resource) {
    f.connect_with([pod]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "s");
}

fn annotated(pod: Resource, default: &str) -> Resource {
    let mut json = pod.into_json();
    json["metadata"]["annotations"] = json!({ DEFAULT_CONTAINER_ANNOTATION: default });
    Resource::from_json(json).expect("a pod")
}

fn names(f: &mut Fixture) -> (Vec<String>, usize) {
    let picker = picker(f).expect("the picker is open");
    f.vcx.update(|_, cx| {
        let picker = picker.read(cx);
        (
            picker
                .choices()
                .containers
                .iter()
                .map(|c| c.name.to_string())
                .collect(),
            picker.selected(),
        )
    })
}

#[gpui::test]
fn several_containers_ask_first_with_the_default_container_preselected(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    press_shell(
        &mut f,
        annotated(pod_with("x", "web-0", &["app", "proxy", "logger"]), "proxy"),
    );
    assert!(
        sent_exec(&f).is_empty(),
        "nothing is sent until the user picks"
    );
    let (containers, selected) = names(&mut f);
    assert_eq!(containers, ["app", "proxy", "logger"]);
    assert_eq!(
        selected, 1,
        "the kubectl.kubernetes.io/default-container annotation"
    );
    // The rows are on screen.
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    for row in [
        "container-picker",
        "container-row-0",
        "container-row-1",
        "container-row-2",
    ] {
        assert!(f.vcx.debug_bounds(row).is_some(), "{row} is drawn");
    }
}

#[gpui::test]
fn without_an_annotation_the_first_container_is_preselected_and_enter_opens_it(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_exec(cx);
    press_shell(&mut f, pod_with("x", "web-0", &["app", "proxy"]));
    assert_eq!(names(&mut f).1, 0);
    f.vcx.simulate_keystrokes("enter");
    f.settle();
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.as_slice(), [Command::PodShell { container: Some(c), .. }] if c == "app"),
        "{sent:?}"
    );
    assert!(picker(&mut f).is_none(), "the picker closed");
}

#[gpui::test]
fn the_arrow_keys_move_the_choice_and_enter_opens_that_container(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    press_shell(&mut f, pod_with("x", "web-0", &["app", "proxy", "logger"]));
    f.vcx.simulate_keystrokes("down down down");
    assert_eq!(names(&mut f).1, 2, "stops at the last one");
    f.vcx.simulate_keystrokes("up");
    assert_eq!(names(&mut f).1, 1);
    f.vcx.simulate_keystrokes("enter");
    f.settle();
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.as_slice(), [Command::PodShell { target, container: Some(c) }]
            if *target == web_ref() && c == "proxy"),
        "{sent:?}"
    );
}

#[gpui::test]
fn escape_cancels_and_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    press_shell(&mut f, pod_with("x", "web-0", &["app", "proxy"]));
    assert!(picker(&mut f).is_some());
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert!(picker(&mut f).is_none(), "closed");
    assert!(
        sent_exec(&f).is_empty(),
        "no command, so no audit record of an open that never was"
    );
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_click_on_a_row_opens_that_container(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    press_shell(&mut f, pod_with("x", "web-0", &["app", "proxy"]));
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = f
        .vcx
        .debug_bounds("container-row-1")
        .expect("a row")
        .center();
    f.vcx.simulate_click(at, Default::default());
    f.settle();
    let sent = sent_exec(&f);
    assert!(
        matches!(sent.as_slice(), [Command::PodShell { container: Some(c), .. }] if c == "proxy"),
        "{sent:?}"
    );
}

#[gpui::test]
fn attach_asks_the_same_question_and_the_last_choice_is_preselected_next_time(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([pod_with("x", "web-0", &["app", "proxy"])]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    f.keys(&table, "a");
    assert_eq!(names(&mut f).0, ["app", "proxy"]);
    f.vcx.simulate_keystrokes("escape");
    f.settle();

    // The service remembers the container opened last in this pod (the terminal's launcher tells
    // it); the picker then starts on it.
    let service = f
        .deps
        .actions
        .as_ref()
        .and_then(|a| a.exec_service().cloned())
        .expect("the exec service");
    service.remember(&web_ref(), "proxy");
    f.keys(&table, "s");
    assert_eq!(names(&mut f).1, 1, "the last choice for this pod");
}
