//! The key-down listener a focused [`TerminalElement`](crate::TerminalElement) registers.
//!
//! GPUI matches the keymap first (`cmd-c`, `ctrl-shift-v`, the window's own shortcuts); a
//! keystroke no binding took reaches [`handle_key_down`]:
//!
//! 1. while an input method is composing, nothing here: the keys belong to it;
//! 2. Shift-PageUp / PageDown / Home / End scroll the history on the primary screen (they are
//!    the terminal's, never sent);
//! 3. [`to_esc_str`](crate::mappings::to_esc_str) maps what has no text of its own (arrows,
//!    function keys, Enter, Ctrl-letters, Alt as meta ...) and the bytes go to the process;
//! 4. anything else is text (`a`, `é`, an IME commit) and is left to the platform's input
//!    handler, which delivers it to [`TerminalState`]'s `EntityInputHandler`.
//!
//! The mapping returns `&'static str` for every common key, so a keypress allocates nothing.

use gpui::{App, Entity, KeyDownEvent, Keystroke};

use crate::grid::{TerminalModes, TerminalScroll};
use crate::settings::TerminalSettings;
use crate::state::TerminalState;

/// The history scroll a Shift + navigation key asks for.
fn history_scroll(keystroke: &Keystroke) -> Option<TerminalScroll> {
    let modifiers = &keystroke.modifiers;
    if !modifiers.shift || modifiers.control || modifiers.alt || modifiers.platform {
        return None;
    }
    match keystroke.key.as_str() {
        "pageup" => Some(TerminalScroll::PageUp),
        "pagedown" => Some(TerminalScroll::PageDown),
        "home" => Some(TerminalScroll::Top),
        "end" => Some(TerminalScroll::Bottom),
        _ => None,
    }
}

/// Handles one key-down for `terminal`. Stops propagation when the key was the terminal's.
pub(crate) fn handle_key_down(
    terminal: &Entity<TerminalState>,
    event: &KeyDownEvent,
    cx: &mut App,
) {
    if terminal.read(cx).is_composing() {
        return;
    }
    let keystroke = &event.keystroke;
    if let Some(scroll) = history_scroll(keystroke)
        && !terminal
            .read(cx)
            .modes()
            .contains(TerminalModes::ALT_SCREEN)
    {
        terminal.update(cx, |terminal, cx| terminal.scroll(scroll, cx));
        cx.stop_propagation();
        return;
    }
    let option_as_meta = TerminalSettings::option_as_meta(cx);
    let sent = terminal.update(cx, |terminal, cx| {
        terminal.type_keystroke(keystroke, option_as_meta, cx)
    });
    if sent {
        cx.stop_propagation();
    }
}
