//! Writing the keymap editor's edits (E21-S08): the user's `keymap.json` keeps its comments, the
//! new binding works at once, an edit that would not validate is never written, and the watcher's
//! echo of the app's own write changes nothing (no reload loop).

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::*;
use gpui::TestAppContext;
use oxikube_domain::command::{KeymapBindingEdit, KeymapBindingRef, KeymapSource};
use oxikube_keymap::edit::{remove_binding_operations, set_binding_operations, write_user_keymap};
use oxikube_keymap::{KeymapOptions, KeymapPlatform, init_with_dir, reload, subscribe_changes};

const USER: &str = "// my keys\n[\n  // the probe\n  {\n    \"context\": \"Pane\",\n    \"bindings\": {\n      \"ctrl-a\": \"kmtest::Alpha\" // first\n    }\n  }\n]\n";

fn mac() -> KeymapOptions {
    KeymapOptions {
        platform: KeymapPlatform::MacOs,
        base_keymap: oxikube_keymap::BaseKeymap::Default,
    }
}

fn set(action: &str, context: Option<&str>, keys: &str) -> KeymapBindingEdit {
    KeymapBindingEdit {
        action: action.to_owned(),
        data: None,
        context: context.map(str::to_owned),
        keystrokes: keys.to_owned(),
        replaces: None,
        unbind: Vec::new(),
    }
}

fn write(cx: &mut TestAppContext, edit: &KeymapBindingEdit) -> Result<(), String> {
    let operations = set_binding_operations(edit).map_err(|err| err.to_string())?;
    let task = cx.update(|cx| write_user_keymap(cx, operations));
    cx.run_until_parked();
    futures::executor::block_on(task).map_err(|err| err.to_string())
}

#[gpui::test]
fn a_saved_binding_is_written_with_the_comments_and_works_at_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, USER).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    let (window, log) = probe(cx, "Pane");
    assert!(press(cx, window, &log, "cmd-shift-k").is_empty());

    write(cx, &set("kmtest::Beta", Some("Pane"), "cmd-shift-k")).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    for kept in [
        "// my keys",
        "// the probe",
        "// first",
        "\"ctrl-a\": \"kmtest::Alpha\"",
    ] {
        assert!(text.contains(kept), "{kept} lost:\n{text}");
    }
    assert!(text.contains("\"cmd-shift-k\": \"kmtest::Beta\""), "{text}");
    assert_eq!(
        press(cx, window, &log, "cmd-shift-k"),
        ["Beta"],
        "no restart"
    );
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Alpha"]);

    // Replace the user binding in place, then remove it.
    let mut edit = set("kmtest::Beta", Some("Pane"), "cmd-shift-j");
    edit.replaces = Some(KeymapBindingRef {
        context: Some("Pane".into()),
        keystrokes: "cmd-shift-k".into(),
        source: KeymapSource::User,
    });
    write(cx, &edit).unwrap();
    assert!(press(cx, window, &log, "cmd-shift-k").is_empty());
    assert_eq!(press(cx, window, &log, "cmd-shift-j"), ["Beta"]);

    let operations = remove_binding_operations(&KeymapBindingRef {
        context: Some("Pane".into()),
        keystrokes: "cmd-shift-j".into(),
        source: KeymapSource::User,
    });
    let task = cx.update(|cx| write_user_keymap(cx, operations));
    cx.run_until_parked();
    futures::executor::block_on(task).unwrap();
    assert!(press(cx, window, &log, "cmd-shift-j").is_empty());
    assert!(std::fs::read_to_string(&path).unwrap().contains("// first"));
}

#[gpui::test]
fn an_edit_that_does_not_validate_is_not_written(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, USER).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));

    let err = write(
        cx,
        &set("kmtest::NoSuchAction", Some("Pane"), "cmd-shift-k"),
    )
    .unwrap_err();
    assert!(err.contains("not written"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        USER,
        "the file is untouched"
    );

    // A binding the file no longer has is refused, not invented.
    let mut gone = set("kmtest::Beta", Some("Pane"), "cmd-shift-j");
    gone.replaces = Some(KeymapBindingRef {
        context: Some("Pane".into()),
        keystrokes: "cmd-shift-q".into(),
        source: KeymapSource::User,
    });
    assert!(write(cx, &gone).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), USER);

    // A file that is not a list cannot be edited.
    std::fs::write(&path, "{ \"oops\": true }").unwrap();
    assert!(write(cx, &set("kmtest::Beta", None, "cmd-shift-k")).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{ \"oops\": true }"
    );
}

#[gpui::test]
fn the_watchers_echo_of_the_apps_own_write_changes_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, USER).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    let changes = Rc::new(Cell::new(0));
    let _subscription = cx.update(|cx| {
        let changes = changes.clone();
        subscribe_changes(cx, move |_| changes.set(changes.get() + 1))
    });

    write(cx, &set("kmtest::Gamma", None, "cmd-shift-g")).unwrap();
    assert_eq!(changes.get(), 1, "the write put the binding in force once");
    let written = std::fs::read_to_string(&path).unwrap();

    // The watcher reads the file after the write and hands it over, as `reload` does.
    cx.update(reload);
    cx.update(reload);
    cx.run_until_parked();
    assert_eq!(changes.get(), 1, "the echo rebinds nothing");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        written,
        "and writes nothing"
    );
}

#[gpui::test]
fn removing_a_default_binding_unbinds_it_in_the_user_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    let (window, log) = probe(cx, "Workspace");
    // The macOS defaults bind `cmd-q` to `app::Quit` everywhere.
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Quit"]);

    let operations = remove_binding_operations(&KeymapBindingRef {
        context: None,
        keystrokes: "cmd-q".into(),
        source: KeymapSource::Default,
    });
    let task = cx.update(|cx| write_user_keymap(cx, operations));
    cx.run_until_parked();
    futures::executor::block_on(task).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"cmd-q\": null"), "{text}");
    assert!(
        press(cx, window, &log, "cmd-q").is_empty(),
        "unbound at once"
    );
    let defaults = oxikube_assets::default_keymap(oxikube_assets::KeymapPlatform::MacOs);
    assert!(defaults.contains("cmd-q"), "the default file is not edited");
}
