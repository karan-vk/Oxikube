//! Keystrokes as the recorder writes them: what GPUI reports, with the platform's own modifier
//! names (`cmd-shift-k` on macOS, `ctrl-shift-k` / `super-k` on Linux, `win-k` on Windows).
//!
//! GPUI's `Keystroke::unparse` names the platform modifier after the OS the binary was built for;
//! here the OS is a parameter, so the spelling of every OS is tested on one machine.

use gpui::Keystroke;
use oxikube_assets::KeymapPlatform;

/// The keys GPUI reports on their own when a modifier is pressed and released alone. The
/// recorder does not bind them: a modifier is part of the next keystroke.
const MODIFIER_KEYS: &[&str] = &[
    "shift", "control", "alt", "platform", "function", "cmd", "ctrl", "super", "win",
];

/// Whether `keystroke` is a modifier pressed and released alone.
pub fn is_modifier_only(keystroke: &Keystroke) -> bool {
    keystroke.key.is_empty() || MODIFIER_KEYS.contains(&keystroke.key.as_str())
}

/// `keystroke` in `keymap.json` syntax with `platform`'s modifier names, modifiers in GPUI's
/// order (`fn`, `ctrl`, `alt`, the platform key, `shift`). A shifted symbol keeps the key GPUI
/// reports (`shift-2`, not `@`), so the binding means the same physical key as the default
/// keymaps; a section with `use_key_equivalents` is where layout-following keys belong.
pub fn keystroke_text(keystroke: &Keystroke, platform: KeymapPlatform) -> String {
    let modifiers = &keystroke.modifiers;
    let mut text = String::new();
    if modifiers.function {
        text.push_str("fn-");
    }
    if modifiers.control {
        text.push_str("ctrl-");
    }
    if modifiers.alt {
        text.push_str("alt-");
    }
    if modifiers.platform {
        text.push_str(match platform {
            KeymapPlatform::MacOs => "cmd-",
            KeymapPlatform::Linux => "super-",
            KeymapPlatform::Windows => "win-",
        });
    }
    if modifiers.shift {
        text.push_str("shift-");
    }
    text.push_str(&keystroke.key);
    text
}

/// A recorded sequence, space separated (`g g`, `cmd-k cmd-s`).
pub fn sequence_text(keystrokes: &[Keystroke], platform: KeymapPlatform) -> String {
    keystrokes
        .iter()
        .map(|keystroke| keystroke_text(keystroke, platform))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    fn stroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_owned(),
            key_char: None,
        }
    }

    #[test]
    fn the_platform_key_is_named_per_os() {
        let k = stroke(
            "k",
            Modifiers {
                platform: true,
                shift: true,
                ..Modifiers::default()
            },
        );
        assert_eq!(keystroke_text(&k, KeymapPlatform::MacOs), "cmd-shift-k");
        assert_eq!(keystroke_text(&k, KeymapPlatform::Linux), "super-shift-k");
        assert_eq!(keystroke_text(&k, KeymapPlatform::Windows), "win-shift-k");
        let ctrl = stroke(
            "k",
            Modifiers {
                control: true,
                shift: true,
                ..Modifiers::default()
            },
        );
        assert_eq!(keystroke_text(&ctrl, KeymapPlatform::Linux), "ctrl-shift-k");
        assert_eq!(keystroke_text(&ctrl, KeymapPlatform::MacOs), "ctrl-shift-k");
    }

    #[test]
    fn shifted_symbols_function_keys_and_sequences_round_trip_through_the_parser() {
        for source in [
            "shift-2",
            "f5",
            "shift-f12",
            "alt-shift-/",
            "fn-left",
            "ctrl-alt-delete",
        ] {
            let parsed = Keystroke::parse(source).unwrap();
            let text = keystroke_text(&parsed, KeymapPlatform::Linux);
            assert_eq!(text, source);
            assert_eq!(Keystroke::parse(&text).unwrap(), parsed, "{source}");
        }
        let g = Keystroke::parse("g").unwrap();
        assert_eq!(sequence_text(&[g.clone(), g], KeymapPlatform::MacOs), "g g");
    }

    #[test]
    fn a_modifier_alone_is_not_a_binding() {
        assert!(is_modifier_only(&stroke("shift", Modifiers::default())));
        assert!(is_modifier_only(&stroke("platform", Modifiers::default())));
        assert!(!is_modifier_only(&Keystroke::parse("cmd-k").unwrap()));
    }
}
