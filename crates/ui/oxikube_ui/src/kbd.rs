//! Key caps: how a keybinding is drawn next to a command, a menu item or a hint.

use gpui::{Action, AsKeystroke as _, KeyContext, Keystroke, Window};
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

/// The first keystroke of the strongest binding of `action` when the key context `context` is in
/// force (`None`: a binding without a context predicate), read from the installed keymap.
pub fn binding_keystroke(
    action: &dyn Action,
    context: Option<&str>,
    window: &Window,
) -> Option<Keystroke> {
    let binding = match context {
        Some(context) => window.highest_precedence_binding_for_action_in_context(
            action,
            KeyContext::parse(context).ok()?,
        ),
        None => {
            window.highest_precedence_binding_for_action_in_context(action, KeyContext::default())
        }
    }?;
    binding
        .keystrokes()
        .first()
        .map(|stroke| stroke.as_keystroke().clone())
}

/// The key cap of [`binding_keystroke`]: what a tooltip shows next to its title
/// ([`crate::tooltip::tooltip_for_action`]).
pub fn binding_hint(action: &dyn Action, context: Option<&str>, window: &Window) -> Option<Kbd> {
    binding_keystroke(action, context, window).map(Kbd::new)
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
