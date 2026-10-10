//! The user's `keymap.json` over a real table (E11-S08): rebinding `y` is honoured in
//! `ResourceTable`, `null` unbinds it, and reloading puts the shipped key back.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};

use super::fixture::{Fixture, cluster};
use super::p;
use crate::table::ResourceTable;

fn apple() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "apple-1")
}

fn open(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "apple-1", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    (f, table)
}

fn user_keymap(f: &mut Fixture, text: &str) {
    f.vcx
        .update(|_, cx| oxikube_keymap::reload_user_keymap(cx, text));
}

fn describe() -> Command {
    Command::ResourceViewDescribe { target: apple() }
}

fn yaml() -> Command {
    Command::ResourceViewYaml { target: apple() }
}

#[gpui::test]
fn a_user_rebinding_of_y_is_honoured_in_the_table(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "y");
    assert_eq!(f.dispatcher.sent(), [yaml()], "the shipped key");
    f.dispatcher.clear();

    user_keymap(
        &mut f,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {"y": "resource_table::ViewDescribe"}}]"#,
    );
    f.keys(&table, "y");
    assert_eq!(f.dispatcher.sent(), [describe()], "y now describes");

    // Back to the shipped key when the user's rebinding goes.
    f.dispatcher.clear();
    user_keymap(&mut f, "[]");
    f.keys(&table, "y");
    assert_eq!(f.dispatcher.sent(), [yaml()]);
}

#[gpui::test]
fn null_unbinds_y_and_the_action_moves_to_a_new_key(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    user_keymap(
        &mut f,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {
            "y": null,
            "ctrl-alt-y": "resource_table::ViewYaml"
        }}]"#,
    );
    f.keys(&table, "y");
    assert_eq!(f.dispatcher.sent(), [], "y does nothing now");
    f.keys(&table, "ctrl-alt-y");
    assert_eq!(f.dispatcher.sent(), [yaml()], "the new key does what y did");
    // The neighbouring shipped verbs are untouched.
    f.dispatcher.clear();
    f.keys(&table, "d");
    assert_eq!(f.dispatcher.sent(), [describe()]);
}

#[gpui::test]
fn a_bad_binding_in_the_file_does_not_stop_the_good_ones(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    user_keymap(
        &mut f,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {
            "ctrl-alt-q": "no_such::Action",
            "y": "resource_table::ViewDescribe"
        }}]"#,
    );
    f.keys(&table, "y");
    assert_eq!(f.dispatcher.sent(), [describe()]);
    let problems = f.vcx.update(|_, cx| oxikube_keymap::user_diagnostics(cx));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].keystrokes.as_deref(), Some("ctrl-alt-q"));
}
