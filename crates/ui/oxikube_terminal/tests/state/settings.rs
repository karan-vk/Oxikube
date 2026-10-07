//! The scrollback limit comes from `terminal.scrollback_lines` and follows hot reloads.

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use oxikube_testkit::fakes::FakeTerminalBackend;

use super::{harness, next_frame};

fn set_user_settings(cx: &mut TestAppContext, text: &str) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| store.set_user_settings(text).unwrap())
    });
}

#[gpui::test]
fn scrollback_limit_comes_from_settings_and_hot_reloads(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        cx.set_global(store);
        oxikube_terminal::init(cx);
    });
    set_user_settings(cx, r#"{ "terminal": { "scrollback_lines": 5 } }"#);

    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    for line in 0..50 {
        backend.output(format!("line {line}\r\n"));
    }
    next_frame(cx);
    let history = h.terminal.read_with(cx, |t, _| t.snapshot().history_size);
    assert_eq!(history, 5);

    // Raising the limit keeps more from now on, without reopening the terminal.
    set_user_settings(cx, r#"{ "terminal": { "scrollback_lines": 20 } }"#);
    for line in 0..50 {
        backend.output(format!("more {line}\r\n"));
    }
    next_frame(cx);
    let history = h.terminal.read_with(cx, |t, _| t.snapshot().history_size);
    assert_eq!(history, 20);

    // Lowering it drops the oldest lines at once.
    set_user_settings(cx, r#"{ "terminal": { "scrollback_lines": 2 } }"#);
    cx.run_until_parked();
    let history = h.terminal.read_with(cx, |t, _| t.snapshot().history_size);
    assert_eq!(history, 2);
}

#[gpui::test]
fn without_a_settings_store_the_default_applies(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    for line in 0..200 {
        backend.output(format!("{line}\r\n"));
    }
    next_frame(cx);
    let history = h.terminal.read_with(cx, |t, _| t.snapshot().history_size);
    assert_eq!(history, 198);
}
