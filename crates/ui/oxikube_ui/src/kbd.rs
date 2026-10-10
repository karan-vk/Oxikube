//! Key caps: how a keybinding is drawn next to a command, a menu item or a hint.

use gpui::Keystroke;
pub use gpui_component::kbd::Kbd;

/// The key cap of one keystroke written as in `keymap.json` (`cmd-shift-p`), drawn with the
/// platform's own symbols (`⌘⇧P` on macOS, `Ctrl+Shift+P` elsewhere). `None` when the text is not
/// a keystroke.
pub fn keycap(keystroke: &str) -> Option<Kbd> {
    Keystroke::parse(keystroke)
        .ok()
        .filter(|stroke| !stroke.key.is_empty())
        .map(Kbd::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keymap_keystroke_is_a_key_cap() {
        assert!(keycap("cmd-shift-p").is_some());
        assert!(keycap("ctrl-k").is_some());
        assert!(keycap("").is_none());
    }

    #[test]
    fn the_platform_symbols_follow_the_os() {
        let stroke = Keystroke::parse("cmd-shift-p").unwrap();
        let text = Kbd::format(&stroke);
        if cfg!(target_os = "macos") {
            assert_eq!(text, "⇧⌘P");
        } else {
            assert!(text.contains("Shift"), "{text}");
        }
    }
}
