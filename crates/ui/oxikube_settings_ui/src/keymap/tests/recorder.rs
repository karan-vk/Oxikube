//! The key recorder in a workspace window over a real keymap (E21-S08): keys are captured with
//! the platform's names and reach no binding, sequences end after a pause, conflicts are listed
//! and answered, an invalid edit cannot be saved, and a saved binding is written to
//! `keymap.json` (comments kept) and works at once.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use gpui::{AppContext as _, Entity, Keystroke, TestAppContext};
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, KeymapSource};
use oxikube_keymap::edit::keystroke_text;
use oxikube_keymap::{KeymapOptions, KeymapPlatform, OpenUser, ShowEditor};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::CommandDispatcher;

use super::{Harness, RecordingBus};
use crate::keymap::recorder::{
    CHECK_DEBOUNCE, ConflictChoice, IDLE_TIMEOUT, KeybindRecorder, Verdict,
};
use crate::keymap::{KeymapEditSink, KeymapEditorRequestSink, KeymapEdits, register_commands};

const USER: &str = r#"// my keys
[
  {
    // the editor's own
    "bindings": { "ctrl-alt-shift-k": "keymap::OpenUser" }
  }
]
"#;

/// Counts `keymap::OpenUser` / `keymap::ShowEditor` reaching the app.
fn count_actions(h: &mut Harness) -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let open = Rc::new(Cell::new(0));
    let show = Rc::new(Cell::new(0));
    h.vcx.update(|_, cx| {
        let o = open.clone();
        cx.on_action(move |_: &OpenUser, _| o.set(o.get() + 1));
        let s = show.clone();
        cx.on_action(move |_: &ShowEditor, _| s.set(s.get() + 1));
    });
    (open, show)
}

/// Select the first row the search `query` finds and open the recorder on it.
fn record(h: &mut Harness, query: &str) -> Entity<KeybindRecorder> {
    let view = h.view.clone();
    h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_query(query, cx);
            v.select(0, cx);
        });
    });
    h.settle();
    let recorder = h
        .vcx
        .update(|window, cx| view.update(cx, |v, cx| v.record_selected(window, cx)))
        .expect("a row is selected and the page is in a workspace");
    h.settle();
    recorder
}

fn keys(h: &mut Harness, recorder: &Entity<KeybindRecorder>) -> Vec<String> {
    h.vcx
        .read(|cx| recorder.read(cx).model().keystrokes().to_vec())
}

fn after(h: &mut Harness, wait: Duration) {
    h.vcx.executor().advance_clock(wait);
    h.settle();
}

fn native(source: &str) -> String {
    keystroke_text(
        &Keystroke::parse(source).expect("a keystroke"),
        KeymapPlatform::current(),
    )
}

#[gpui::test]
fn recording_captures_platform_names_and_no_binding_fires(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let (open, _) = count_actions(&mut h);
    h.vcx.simulate_keystrokes("ctrl-alt-shift-k");
    h.settle();
    assert_eq!(open.get(), 1, "the binding works before recording");

    let recorder = record(&mut h, "keymap::OpenUser");
    assert!(h.is_laid_out("keymap-recorder"));
    h.vcx.simulate_keystrokes("ctrl-alt-shift-k");
    h.vcx.simulate_keystrokes("escape");
    h.settle();
    assert_eq!(
        open.get(),
        1,
        "no keystroke reached a binding while recording"
    );
    assert_eq!(keys(&mut h, &recorder), ["ctrl-alt-shift-k", "escape"]);
    assert!(
        h.is_laid_out("keymap-recorder"),
        "Escape is recorded, not a close"
    );
    assert!(h.is_laid_out("keymap-recorder-keys"));

    // The platform key is written with the platform's name: `cmd` on macOS, `super` on Linux.
    h.vcx
        .update(|_, cx| recorder.update(cx, |r, cx| r.clear(cx)));
    h.vcx
        .update(|window, cx| recorder.update(cx, |r, cx| r.restart(window, cx)));
    h.vcx.simulate_keystrokes("cmd-shift-k");
    h.settle();
    assert_eq!(keys(&mut h, &recorder), [native("cmd-shift-k")]);
    if cfg!(target_os = "macos") {
        assert_eq!(keys(&mut h, &recorder), ["cmd-shift-k"]);
    }
}

#[gpui::test]
fn each_platform_writes_its_own_modifier_names(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let row = h.read(|v, cx| {
        v.table().read(cx, |d| {
            let m = d.model();
            (0..m.len())
                .filter_map(|i| m.row(i))
                .find(|r| r.action == "keymap::OpenUser")
                .cloned()
                .expect("the user's row")
        })
    });
    for (platform, typed, written) in [
        (KeymapPlatform::MacOs, "cmd-shift-k", "cmd-shift-k"),
        (KeymapPlatform::Linux, "ctrl-shift-k", "ctrl-shift-k"),
        (KeymapPlatform::Windows, "ctrl-alt-f5", "ctrl-alt-f5"),
    ] {
        let bus: Rc<dyn CommandDispatcher> = Rc::new(RecordingBus::default());
        let row = row.clone();
        let recorder = h.vcx.update(|window, cx| {
            let recorder =
                cx.new(|cx| KeybindRecorder::new(&row, platform, bus.clone(), window, cx));
            h.workspace
                .update(cx, |ws, cx| ws.show_modal(recorder.clone(), window, cx));
            recorder
        });
        h.settle();
        h.vcx.simulate_keystrokes(typed);
        h.settle();
        assert_eq!(keys(&mut h, &recorder), [written], "{platform:?}");
    }
}

#[gpui::test]
fn a_sequence_ends_after_a_pause_and_keys_then_work_again(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let (open, _) = count_actions(&mut h);
    let recorder = record(&mut h, "keymap::OpenUser");
    h.vcx.simulate_keystrokes("g");
    after(&mut h, IDLE_TIMEOUT / 2);
    h.vcx.simulate_keystrokes("g");
    h.settle();
    assert_eq!(keys(&mut h, &recorder), ["g", "g"]);
    assert!(h.vcx.read(|cx| recorder.read(cx).model().is_recording()));
    after(&mut h, IDLE_TIMEOUT);
    assert!(!h.vcx.read(|cx| recorder.read(cx).model().is_recording()));
    h.vcx.simulate_keystrokes("ctrl-alt-shift-k");
    h.settle();
    assert_eq!(
        keys(&mut h, &recorder),
        ["g", "g"],
        "not recording any more"
    );
    assert_eq!(open.get(), 1, "the key reaches its binding again");
}

#[gpui::test]
fn conflicts_are_listed_and_replace_unbinds_them(cx: &mut TestAppContext) {
    let user = r#"[
      { "context": "Workspace", "bindings": { "ctrl-alt-shift-j": "keymap::OpenUser" } }
    ]"#;
    let mut h = Harness::open(cx, user);
    // A command bound nowhere: the recorder adds a binding for it.
    let recorder = record(&mut h, "keymap::ShowEditor");
    h.vcx.update(|window, cx| {
        recorder.update(cx, |r, cx| r.type_context("ResourceTable", window, cx));
    });
    h.vcx.simulate_keystrokes("ctrl-alt-shift-j");
    after(&mut h, IDLE_TIMEOUT);
    let conflicts = h
        .vcx
        .read(|cx| recorder.read(cx).model().conflicts().to_vec());
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].row.action, "keymap::OpenUser");
    assert!(h.is_laid_out("keymap-recorder-conflicts"));

    h.click("keymap-recorder-replace");
    assert_eq!(
        h.vcx.read(|cx| recorder.read(cx).model().choice()),
        ConflictChoice::Replace
    );
    after(&mut h, CHECK_DEBOUNCE);
    assert!(h.vcx.read(|cx| recorder.read(cx).model().can_save()));
    h.click("keymap-recorder-save");
    let sent = h.bus.sent();
    let Some(Command::KeymapSetBinding { edit }) = sent.last() else {
        panic!("{sent:?}");
    };
    assert_eq!(edit.action, "keymap::ShowEditor");
    assert_eq!(edit.context.as_deref(), Some("ResourceTable"));
    assert_eq!(edit.keystrokes, "ctrl-alt-shift-j");
    assert_eq!(edit.unbind.len(), 1);
    assert_eq!(edit.unbind[0].source, KeymapSource::User);
    assert!(
        !h.is_laid_out("keymap-recorder"),
        "saving closes the recorder"
    );
}

#[gpui::test]
fn an_invalid_edit_shows_its_errors_and_cannot_be_saved(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let recorder = record(&mut h, "keymap::OpenUser");
    h.vcx.simulate_keystrokes("ctrl-alt-shift-y");
    h.vcx.update(|_, cx| {
        recorder.update(cx, |r, cx| r.set_context("ResourceTable &&", cx));
    });
    after(&mut h, CHECK_DEBOUNCE);
    let verdict = h.vcx.read(|cx| recorder.read(cx).model().verdict().clone());
    assert!(
        matches!(verdict, Verdict::Blocked(ref errors) if !errors.is_empty()),
        "{verdict:?}"
    );
    assert!(h.is_laid_out("keymap-recorder-problems"));
    h.click("keymap-recorder-save");
    assert!(h.bus.sent().is_empty(), "nothing is sent");
    assert!(h.is_laid_out("keymap-recorder"), "the recorder stays open");

    // Fixing the context makes it savable.
    h.vcx
        .update(|_, cx| recorder.update(cx, |r, cx| r.set_context("ResourceTable", cx)));
    after(&mut h, CHECK_DEBOUNCE);
    assert!(h.vcx.read(|cx| recorder.read(cx).model().can_save()));
}

#[gpui::test]
fn remove_sends_the_selected_binding_and_skips_rows_out_of_force(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, USER);
    let view = h.view.clone();
    let removed = h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_query("ctrl-alt-shift-k", cx);
            v.select(0, cx);
            v.remove_selected(cx)
        })
    });
    assert!(removed);
    let sent = h.bus.sent();
    let Some(Command::KeymapRemoveBinding { binding }) = sent.last() else {
        panic!("{sent:?}");
    };
    assert_eq!(binding.keystrokes, "ctrl-alt-shift-k");
    assert_eq!(binding.source, KeymapSource::User);

    // A command bound nowhere has nothing to remove.
    let removed = h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_query("keymap::ShowEditor", cx);
            v.set_show_unbound(true, cx);
            v.select(0, cx);
            v.remove_selected(cx)
        })
    });
    assert!(!removed);
    assert_eq!(h.bus.sent().len(), 1);
}

#[gpui::test]
fn a_saved_binding_is_written_with_its_comments_and_works_without_restart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, USER).expect("write the user file");
    let config = dir.path().to_owned();
    let mut h = Harness::open_with(cx, move |cx| {
        oxikube_keymap::init_with_dir(&config, KeymapOptions::default(), cx);
    });
    let (open, _) = count_actions(&mut h);

    // The real bus, with the edits applied on the UI thread.
    let (pages, _page_requests) = KeymapEditorRequestSink::channel();
    let (edits, edit_requests) = KeymapEditSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_settings_ui::keymap", |r| {
            register_commands(r, pages, edits)
        })
        .expect("registered");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    );
    let _runner = h.vcx.update(|_, cx| KeymapEdits::start(edit_requests, cx));

    let recorder = record(&mut h, "ctrl-alt-shift-k");
    h.vcx.simulate_keystrokes("ctrl-alt-shift-l");
    after(&mut h, IDLE_TIMEOUT);
    assert!(h.vcx.read(|cx| recorder.read(cx).model().can_save()));
    h.click("keymap-recorder-save");
    let command = h.bus.sent().pop().expect("the recorder sent its edit");
    let dispatch = h.vcx.executor().spawn({
        async move {
            bus.dispatch(
                command,
                DispatchContext::new(Initiator::Ui, "keymap-editor"),
            )
            .await
        }
    });
    h.settle();
    block_on(dispatch).expect("the edit was written");

    let text = std::fs::read_to_string(&path).expect("the user file");
    assert!(
        text.contains("// my keys") && text.contains("// the editor's own"),
        "{text}"
    );
    assert!(
        text.contains("\"ctrl-alt-shift-l\": \"keymap::OpenUser\""),
        "{text}"
    );
    assert!(
        !text.contains("ctrl-alt-shift-k"),
        "replaced in place: {text}"
    );
    h.vcx.simulate_keystrokes("ctrl-alt-shift-l");
    h.settle();
    assert_eq!(open.get(), 1, "the new key works at once");
    h.vcx.simulate_keystrokes("ctrl-alt-shift-k");
    h.settle();
    assert_eq!(open.get(), 1, "the old one is gone");
    let view = h.view.clone();
    h.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.set_query("ctrl-alt-shift-l", cx)));
    let shown = h.shown();
    assert!(
        shown
            .iter()
            .any(|row| row == "keymap::OpenUser [ctrl-alt-shift-l]"),
        "the list followed: {shown:?}"
    );

    // A self-made reload (the watcher's echo) changes nothing more.
    h.vcx.update(|_, cx| oxikube_keymap::reload(cx));
    h.settle();
    assert_eq!(std::fs::read_to_string(&path).expect("the user file"), text);
}
