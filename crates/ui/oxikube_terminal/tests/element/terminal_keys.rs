//! The shipped keymaps' `Terminal` context against a focused element (E09-S11): per platform, the
//! chords reach the terminal's actions, `ctrl-c` and the other plain control chords reach the
//! process, and a user's `keymap.json` rebinds them.

use gpui::{ClipboardItem, TestAppContext};
use oxikube_keymap::{KeymapOptions, KeymapPlatform, init_with_text};

use super::{FONT_SIZE, Harness, harness};

fn install(cx: &mut TestAppContext, platform: KeymapPlatform, user: &str) {
    cx.update(|cx| {
        init_with_text(
            user,
            KeymapOptions {
                platform,
                vim: false,
            },
            cx,
        )
    });
}

fn with_history(cx: &mut TestAppContext) -> Harness {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let lines: String = (0..40).map(|i| format!("line {i}\r\n")).collect();
    h.output(&lines);
    h
}

fn selection(h: &mut Harness) -> Option<String> {
    h.terminal
        .read_with(&mut *h.window, |terminal, _| terminal.selection_text())
}

fn history(h: &mut Harness) -> usize {
    h.terminal.read_with(&mut *h.window, |terminal, _| {
        terminal.snapshot().history_size
    })
}

fn offset(h: &mut Harness) -> usize {
    h.terminal.read_with(&mut *h.window, |terminal, _| {
        terminal.snapshot().display_offset
    })
}

/// The chord of each platform for the terminal's own actions.
struct Chords {
    platform: KeymapPlatform,
    select_all: &'static str,
    clear: &'static str,
    copy: &'static str,
}

const PLATFORMS: [Chords; 3] = [
    Chords {
        platform: KeymapPlatform::MacOs,
        select_all: "cmd-a",
        clear: "cmd-k",
        copy: "cmd-c",
    },
    Chords {
        platform: KeymapPlatform::Linux,
        select_all: "ctrl-shift-a",
        clear: "ctrl-shift-k",
        copy: "ctrl-shift-c",
    },
    Chords {
        platform: KeymapPlatform::Windows,
        select_all: "ctrl-shift-a",
        clear: "ctrl-shift-k",
        copy: "ctrl-shift-c",
    },
];

#[gpui::test]
fn select_all_copy_and_clear_follow_the_platform_chords(cx: &mut TestAppContext) {
    for chords in PLATFORMS {
        install(cx, chords.platform, "");
        let mut h = with_history(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));

        h.window.simulate_keystrokes(chords.select_all);
        let all = selection(&mut h).expect("everything is selected");
        assert!(
            all.starts_with("line 0") && all.contains("line 39"),
            "{all}"
        );
        h.window.simulate_keystrokes(chords.copy);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some(all.as_str()),
            "{:?}: the chord copied the selection",
            chords.platform
        );

        assert!(history(&mut h) > 0);
        h.window.simulate_keystrokes(chords.clear);
        assert_eq!(history(&mut h), 0, "{:?}: cleared", chords.platform);
        assert_eq!(h.written(), b"", "none of it reached the process");
    }
}

#[gpui::test]
fn ctrl_c_and_the_plain_control_chords_reach_the_process_on_every_platform(
    cx: &mut TestAppContext,
) {
    for chords in PLATFORMS {
        install(cx, chords.platform, "");
        let mut h = with_history(cx);
        // A selection exists: ctrl-c is still an interrupt, never a copy.
        h.window.simulate_keystrokes(chords.select_all);
        h.window
            .simulate_keystrokes("ctrl-c ctrl-d ctrl-z ctrl-r ctrl-l ctrl-a ctrl-k ctrl-f");
        assert_eq!(
            h.written(),
            b"\x03\x04\x1a\x12\x0c\x01\x0b\x06",
            "{:?}: plain control chords belong to the shell",
            chords.platform
        );
    }
}

#[gpui::test]
fn the_scroll_chords_move_the_history_and_leave_a_full_screen_program_its_keys(
    cx: &mut TestAppContext,
) {
    for platform in [KeymapPlatform::MacOs, KeymapPlatform::Linux] {
        install(cx, platform, "");
        let mut h = with_history(cx);
        h.window.simulate_keystrokes("shift-pageup");
        assert_eq!(offset(&mut h), 10, "a page up (10 rows)");
        h.window.simulate_keystrokes("shift-pagedown");
        assert_eq!(offset(&mut h), 0);
        h.window.simulate_keystrokes("shift-up shift-up shift-up");
        assert_eq!(offset(&mut h), 3, "a line each");
        h.window.simulate_keystrokes("shift-down");
        assert_eq!(offset(&mut h), 2);
        assert_eq!(h.written(), b"", "the scroll keys are the terminal's");

        // On the alternate screen (vim, less) there is no history: the program gets the keys.
        h.output("\x1b[?1049h");
        h.window.simulate_keystrokes("shift-up");
        assert_eq!(h.written(), b"\x1b[1;2A", "{platform:?}");
    }
}

#[gpui::test]
fn a_user_keymap_rebinds_and_unbinds_the_terminal_chords(cx: &mut TestAppContext) {
    install(
        cx,
        KeymapPlatform::Linux,
        r#"[{ "context": "Terminal", "bindings": {
            "ctrl-shift-k": "terminal::SelectAll",
            "ctrl-shift-a": null,
            "alt-k": "terminal::Clear"
        } }]"#,
    );
    let mut h = with_history(cx);
    h.window.simulate_keystrokes("ctrl-shift-k");
    assert!(selection(&mut h).is_some(), "ctrl-shift-k now selects all");
    assert!(history(&mut h) > 0, "and no longer clears");

    h.window.simulate_keystrokes("alt-k");
    assert_eq!(history(&mut h), 0, "alt-k clears");

    let mut h = with_history(cx);
    h.window.simulate_keystrokes("ctrl-shift-a");
    assert_eq!(selection(&mut h), None, "ctrl-shift-a is unbound");
}
