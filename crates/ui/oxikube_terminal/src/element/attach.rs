//! What the element attaches to the window while it paints (E09-S06): the `Terminal` key context,
//! the input handler (IME and plain text), the key-down listener and the `terminal::Copy` /
//! `terminal::Paste` action handlers. All of it is per frame and active only while the element's
//! focus handle is focused.

use std::any::TypeId;

use gpui::{
    App, Bounds, DispatchPhase, ElementInputHandler, KeyContext, KeyDownEvent, Pixels, Window,
};

use super::TerminalElement;
use crate::input::{self, KEY_CONTEXT, clipboard, keyboard};

/// The key context of a terminal: `Terminal`.
fn key_context() -> KeyContext {
    let mut context = KeyContext::default();
    context.add(KEY_CONTEXT);
    context
}

/// Registers this frame's keyboard, IME and clipboard handling. Call during paint.
pub(super) fn register(
    element: &TerminalElement,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    window.set_key_context(key_context());
    window.handle_input(
        &element.focus,
        ElementInputHandler::new(bounds, element.terminal.clone()),
        cx,
    );

    let terminal = element.terminal.clone();
    window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            keyboard::handle_key_down(&terminal, event, window, cx);
        }
    });
    let terminal = element.terminal.clone();
    window.on_action(TypeId::of::<input::Copy>(), move |_, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            clipboard::copy_selection(&terminal, cx);
        }
    });
    let terminal = element.terminal.clone();
    let confirm = element.confirm.clone();
    window.on_action(TypeId::of::<input::Paste>(), move |_, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            clipboard::paste_clipboard(&terminal, confirm.as_ref(), window, cx);
        }
    });
}
