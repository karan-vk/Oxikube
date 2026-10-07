//! Key family x modifier x mode.

use std::borrow::Cow;

use gpui::Keystroke;

use crate::mappings::{KeyMode, to_esc_str};

fn ks(text: &str) -> Keystroke {
    Keystroke::parse(text).unwrap_or_else(|_| panic!("a keystroke: {text}"))
}

fn plain() -> KeyMode {
    KeyMode::default()
}

fn app() -> KeyMode {
    KeyMode {
        app_cursor: true,
        ..plain()
    }
}

fn meta() -> KeyMode {
    KeyMode {
        option_as_meta: true,
        ..plain()
    }
}

fn map(text: &str, mode: KeyMode) -> Option<String> {
    to_esc_str(&ks(text), mode).map(Cow::into_owned)
}

#[test]
fn cursor_keys_follow_application_cursor_mode() {
    for (key, normal, application) in [
        ("up", "\x1b[A", "\x1bOA"),
        ("down", "\x1b[B", "\x1bOB"),
        ("right", "\x1b[C", "\x1bOC"),
        ("left", "\x1b[D", "\x1bOD"),
        ("home", "\x1b[H", "\x1bOH"),
        ("end", "\x1b[F", "\x1bOF"),
    ] {
        assert_eq!(map(key, plain()).as_deref(), Some(normal), "{key}");
        assert_eq!(
            map(key, app()).as_deref(),
            Some(application),
            "{key} (DECCKM)"
        );
    }
}

#[test]
fn modified_cursor_keys_are_csi_with_the_xterm_parameter_in_both_modes() {
    for (prefix, parameter) in [
        ("shift-", 2),
        ("alt-", 3),
        ("shift-alt-", 4),
        ("ctrl-", 5),
        ("ctrl-shift-", 6),
        ("ctrl-alt-", 7),
        ("ctrl-alt-shift-", 8),
    ] {
        let expected = format!("\x1b[1;{parameter}A");
        for mode in [plain(), app()] {
            assert_eq!(
                map(&format!("{prefix}up"), mode).as_deref(),
                Some(expected.as_str()),
                "{prefix}up"
            );
        }
    }
    assert_eq!(map("ctrl-right", plain()).as_deref(), Some("\x1b[1;5C"));
    assert_eq!(map("shift-home", app()).as_deref(), Some("\x1b[1;2H"));
    assert_eq!(map("alt-end", plain()).as_deref(), Some("\x1b[1;3F"));
}

#[test]
fn editing_and_paging_keys_are_tilde_sequences() {
    for (key, number) in [("insert", 2), ("delete", 3), ("pageup", 5), ("pagedown", 6)] {
        assert_eq!(map(key, plain()), Some(format!("\x1b[{number}~")), "{key}");
        // Application cursor keys do not change them.
        assert_eq!(map(key, app()), Some(format!("\x1b[{number}~")), "{key}");
        assert_eq!(
            map(&format!("ctrl-{key}"), plain()),
            Some(format!("\x1b[{number};5~"))
        );
        assert_eq!(
            map(&format!("shift-alt-{key}"), plain()),
            Some(format!("\x1b[{number};4~"))
        );
    }
}

#[test]
fn function_keys_f1_to_f12() {
    for (index, expected) in ["\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS"].iter().enumerate() {
        assert_eq!(
            map(&format!("f{}", index + 1), plain()).as_deref(),
            Some(*expected)
        );
        assert_eq!(
            map(&format!("f{}", index + 1), app()).as_deref(),
            Some(*expected)
        );
    }
    for (key, number) in [
        (5, 15),
        (6, 17),
        (7, 18),
        (8, 19),
        (9, 20),
        (10, 21),
        (11, 23),
        (12, 24),
    ] {
        assert_eq!(
            map(&format!("f{key}"), plain()),
            Some(format!("\x1b[{number}~")),
            "f{key}"
        );
        assert_eq!(
            map(&format!("shift-f{key}"), plain()),
            Some(format!("\x1b[{number};2~"))
        );
    }
    assert_eq!(map("shift-f1", plain()).as_deref(), Some("\x1b[1;2P"));
    assert_eq!(map("ctrl-f4", plain()).as_deref(), Some("\x1b[1;5S"));
    assert_eq!(map("f13", plain()), None, "beyond F12 is not mapped");
    assert_eq!(map("f0", plain()), None);
}

#[test]
fn enter_escape_tab_and_backspace() {
    assert_eq!(map("enter", plain()).as_deref(), Some("\r"));
    assert_eq!(map("shift-enter", plain()).as_deref(), Some("\r"));
    assert_eq!(map("alt-enter", plain()).as_deref(), Some("\x1b\r"));
    assert_eq!(map("escape", plain()).as_deref(), Some("\x1b"));
    assert_eq!(map("alt-escape", plain()).as_deref(), Some("\x1b\x1b"));
    assert_eq!(map("tab", plain()).as_deref(), Some("\t"));
    assert_eq!(map("shift-tab", plain()).as_deref(), Some("\x1b[Z"));
    assert_eq!(map("alt-tab", plain()).as_deref(), Some("\x1b\t"));
    assert_eq!(map("ctrl-tab", plain()), None, "a window shortcut");
    assert_eq!(map("backspace", plain()).as_deref(), Some("\x7f"));
    assert_eq!(map("shift-backspace", plain()).as_deref(), Some("\x7f"));
    assert_eq!(map("ctrl-backspace", plain()).as_deref(), Some("\x08"));
    assert_eq!(map("alt-backspace", plain()).as_deref(), Some("\x1b\x7f"));
}

#[test]
fn ctrl_letters_are_control_codes() {
    assert_eq!(map("ctrl-a", plain()).as_deref(), Some("\x01"));
    assert_eq!(map("ctrl-c", plain()).as_deref(), Some("\x03"));
    assert_eq!(map("ctrl-d", plain()).as_deref(), Some("\x04"));
    assert_eq!(map("ctrl-z", plain()).as_deref(), Some("\x1a"));
    assert_eq!(map("ctrl-shift-a", plain()).as_deref(), Some("\x01"));
    for (index, letter) in ('a'..='z').enumerate() {
        let expected = char::from(u8::try_from(index + 1).unwrap()).to_string();
        assert_eq!(
            map(&format!("ctrl-{letter}"), plain()),
            Some(expected),
            "{letter}"
        );
    }
    assert_eq!(map("ctrl-[", plain()).as_deref(), Some("\x1b"));
    assert_eq!(map("ctrl-\\", plain()).as_deref(), Some("\x1c"));
    assert_eq!(map("ctrl-]", plain()).as_deref(), Some("\x1d"));
    assert_eq!(map("ctrl-space", plain()).as_deref(), Some("\x00"));
    assert_eq!(map("ctrl-@", plain()).as_deref(), Some("\x00"));
    assert_eq!(map("ctrl-/", plain()).as_deref(), Some("\x1f"));
    assert_eq!(map("ctrl--", plain()).as_deref(), Some("\x1f"), "undo");
    assert_eq!(map("ctrl-?", plain()).as_deref(), Some("\x7f"));
    assert_eq!(map("ctrl-1", plain()), None, "no control code");
}

#[test]
fn alt_prefixes_escape_only_when_it_is_meta() {
    // Off (macOS default): Option composes characters, so it is text for the input handler.
    assert_eq!(map("alt-b", plain()), None);
    assert_eq!(map("alt-space", plain()), None);
    assert_eq!(map("alt-b", meta()).as_deref(), Some("\x1bb"));
    assert_eq!(map("alt-f", meta()).as_deref(), Some("\x1bf"));
    assert_eq!(map("alt-.", meta()).as_deref(), Some("\x1b."));
    assert_eq!(map("alt-space", meta()).as_deref(), Some("\x1b "));
    assert_eq!(map("alt-shift-b", meta()).as_deref(), Some("\x1bB"));
    // Control codes take the prefix regardless of the setting.
    assert_eq!(map("ctrl-alt-a", plain()).as_deref(), Some("\x1b\x01"));
    assert_eq!(map("ctrl-alt-a", meta()).as_deref(), Some("\x1b\x01"));
}

#[test]
fn macos_option_composition_is_not_meta_text() {
    // Option-b on a Mac: the key is "b", the composed character is "∫".
    let composed = Keystroke {
        key_char: Some("∫".into()),
        ..ks("alt-b")
    };
    assert_eq!(
        to_esc_str(&composed, plain()),
        None,
        "left to the input handler"
    );
    assert_eq!(
        to_esc_str(&composed, meta()).as_deref(),
        Some("\x1bb"),
        "as meta the key, not the composed character, is sent"
    );
    // The platform delivered shifted ASCII: it wins.
    let shifted = Keystroke {
        key_char: Some("!".into()),
        ..ks("alt-shift-1")
    };
    assert_eq!(to_esc_str(&shifted, meta()).as_deref(), Some("\x1b!"));
}

#[test]
fn a_non_ascii_meta_key_is_the_only_allocation() {
    assert!(matches!(
        to_esc_str(&ks("alt-b"), meta()),
        Some(Cow::Borrowed(_))
    ));
    assert!(matches!(
        to_esc_str(&ks("up"), plain()),
        Some(Cow::Borrowed(_))
    ));
    assert!(matches!(
        to_esc_str(&ks("ctrl-alt-shift-f9"), plain()),
        Some(Cow::Borrowed(_))
    ));
    assert!(matches!(
        to_esc_str(&ks("ctrl-c"), plain()),
        Some(Cow::Borrowed(_))
    ));
    let accented = Keystroke {
        key: "é".into(),
        key_char: Some("é".into()),
        ..ks("alt-e")
    };
    assert_eq!(to_esc_str(&accented, meta()).as_deref(), Some("\x1bé"));
}

#[test]
fn text_modifiers_and_shortcuts_are_not_terminal_keys() {
    for text in ["a", "shift-a", "1", "space", ".", "shift-1"] {
        assert_eq!(map(text, meta()), None, "{text} is text");
    }
    for text in [
        "cmd-c",
        "cmd-v",
        "cmd-up",
        "cmd-a",
        "cmd-enter",
        "cmd-shift-p",
    ] {
        assert_eq!(map(text, meta()), None, "{text} is an application shortcut");
    }
    let bare = Keystroke {
        key: "shift".into(),
        ..ks("shift-a")
    };
    assert_eq!(to_esc_str(&bare, meta()), None);
    assert_eq!(map("capslock", plain()), None);
}
