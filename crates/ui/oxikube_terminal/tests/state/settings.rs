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

#[gpui::test]
fn several_settings_changed_at_once_notify_the_terminal_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        cx.set_global(store);
        oxikube_terminal::init(cx);
    });
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    cx.run_until_parked();
    let before = h.notifies.get();

    set_user_settings(
        cx,
        r#"{ "terminal": { "font_size": 18, "line_height": 1.6, "cursor_shape": "bar",
                           "cursor_blink": true, "scrollback_lines": 50 } }"#,
    );
    cx.run_until_parked();
    assert_eq!(
        h.notifies.get(),
        before + 1,
        "one repaint for the whole change"
    );
    let cursor = h.terminal.read_with(cx, |t, _| t.snapshot().cursor);
    assert_eq!(cursor.shape, oxikube_terminal::grid::CursorShape::Beam);
    assert!(cursor.blinking);

    // A change to some other setting wakes nothing.
    set_user_settings(
        cx,
        r#"{ "terminal": { "font_size": 18, "line_height": 1.6, "cursor_shape": "bar",
                           "cursor_blink": true, "scrollback_lines": 50 }, "ui_scale": 1.25 }"#,
    );
    cx.run_until_parked();
    assert_eq!(
        h.notifies.get(),
        before + 1,
        "unrelated settings do not repaint terminals"
    );
}

#[gpui::test]
fn a_new_terminal_starts_with_the_configured_cursor(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        cx.set_global(store);
        oxikube_terminal::init(cx);
    });
    set_user_settings(cx, r#"{ "terminal": { "cursor_shape": "underline" } }"#);
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    let cursor = h.terminal.read_with(cx, |t, _| t.snapshot().cursor);
    assert_eq!(cursor.shape, oxikube_terminal::grid::CursorShape::Underline);
    assert!(!cursor.blinking);
}

#[gpui::test]
fn select_all_and_clear_notify_and_change_the_grid(cx: &mut TestAppContext) {
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    for line in 0..10 {
        backend.output(format!("line {line}\r\n"));
    }
    next_frame(cx);
    let before = h.notifies.get();
    h.terminal.update(cx, |t, cx| t.select_all(cx));
    assert!(
        h.terminal
            .read_with(cx, |t, _| t.selection_text())
            .is_some()
    );
    h.terminal.update(cx, |t, cx| t.clear(cx));
    assert_eq!(h.notifies.get(), before + 2);
    assert_eq!(
        h.terminal.read_with(cx, |t, _| t.snapshot().history_size),
        0
    );
    assert_eq!(h.terminal.read_with(cx, |t, _| t.selection_text()), None);
}

#[gpui::test]
fn the_content_generation_moves_when_a_clear_a_resize_or_a_smaller_scrollback_moves_lines(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        cx.set_global(store);
        oxikube_terminal::init(cx);
    });
    let backend = FakeTerminalBackend::silent();
    let h = harness(cx, &backend, (20, 3));
    for line in 0..20 {
        backend.output(format!("line {line}\r\n"));
    }
    next_frame(cx);
    let generation = |h: &super::Harness, cx: &mut TestAppContext| {
        h.terminal.read_with(cx, |t, _| t.content_generation())
    };
    let mut last = generation(&h, cx);

    h.terminal.update(cx, |t, cx| {
        t.scroll(oxikube_terminal::grid::TerminalScroll::Top, cx);
        t.select_all(cx);
    });
    assert_eq!(generation(&h, cx), last, "scrolling and selecting keep it");

    h.terminal.update(cx, |t, cx| t.clear(cx));
    assert_ne!(generation(&h, cx), last, "a clear drops the history");
    last = generation(&h, cx);

    h.terminal.update(cx, |t, cx| {
        t.resize(oxikube_ports::TerminalSize::new(10, 3), cx)
    });
    assert_ne!(generation(&h, cx), last, "a resize reflows the lines");
    last = generation(&h, cx);

    h.terminal.update(cx, |t, cx| {
        t.resize(oxikube_ports::TerminalSize::new(10, 3), cx)
    });
    assert_eq!(
        generation(&h, cx),
        last,
        "the same size again moves nothing"
    );

    set_user_settings(cx, r#"{ "terminal": { "scrollback_lines": 2 } }"#);
    cx.run_until_parked();
    assert_ne!(generation(&h, cx), last, "a smaller scrollback drops lines");
}
