//! The completions under the line, Tab, and Enter on a completion the user chose.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{Fixture, wait_for_data};

#[gpui::test]
fn typing_narrows_the_completions_to_the_word_under_the_caret(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    f.type_text("dep");
    let listed = f.read(|d| d.completions());
    assert!(listed.len() < 40, "narrowed: {}", listed.len());
    assert_eq!(listed.first().map(String::as_str), Some("deploy"));
    assert!(listed.iter().all(|w| w.contains('d')));
}

#[gpui::test]
fn after_the_alias_the_completions_are_the_namespaces(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    wait_for_data(&mut f);
    f.type_text("pods ");
    assert_eq!(
        f.read(|d| d.completions()),
        ["all", "default", "kube-system", "monitoring", "web"]
    );
    f.type_text("kube");
    assert_eq!(f.read(|d| d.completions()), ["kube-system"]);
}

#[gpui::test]
fn after_an_at_sign_the_completions_are_the_contexts(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    wait_for_data(&mut f);
    f.type_text("pods @st");
    assert_eq!(f.read(|d| d.completions()), ["staging"]);
}

#[gpui::test]
fn tab_takes_the_selected_completion_and_leaves_room_for_the_next_word(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    f.type_text("dep");
    f.keys("tab");
    assert_eq!(f.query(), "deploy ");
    assert!(f.bar().is_some(), "Tab does not run the line");
    // The next word now completes namespaces.
    wait_for_data(&mut f);
    f.type_text("kube");
    f.keys("tab");
    assert_eq!(f.query(), "deploy kube-system ");
}

#[gpui::test]
fn enter_on_a_completion_chosen_with_the_arrows_takes_it_instead_of_running(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    f.open();
    f.type_text("dep");
    f.keys("down");
    let second = f.read(|d| d.completions()[1].clone());
    f.keys("enter");
    assert_eq!(f.query(), format!("{second} "));
    assert!(f.bar().is_some());
    assert_eq!(f.take_sent(), []);
}

#[gpui::test]
fn enter_runs_the_typed_line_when_no_completion_was_chosen(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    f.type_text("po");
    assert!(f.read(|d| d.completions().len()) > 1);
    f.keys("enter");
    // `po` is itself the pods alias, whatever else matches it.
    let sent = f.take_sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceOpenList { gvk, .. }] if &*gvk.kind == "Pod")
    );
}
