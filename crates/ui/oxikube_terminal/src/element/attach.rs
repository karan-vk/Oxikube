//! What the element attaches to the window while it paints (E09-S06): the `Terminal` key context,
//! the input handler (IME and plain text), the key-down listener and the `terminal::Copy` /
//! `terminal::Paste` action handlers. All of it is per frame and active only while the element's
//! focus handle is focused.

use std::any::TypeId;

use gpui::{
    App, Bounds, DispatchPhase, ElementInputHandler, KeyContext, KeyDownEvent, Pixels, Window,
};

use super::TerminalElement;
use crate::grid::{TerminalModes, TerminalScroll};
use crate::input::{self, KEY_CONTEXT, clipboard, keyboard};

/// The key context of a terminal: `Terminal`.
fn key_context(searching: bool) -> KeyContext {
    let mut context = KeyContext::default();
    context.add(KEY_CONTEXT);
    if searching {
        context.add("searching");
    }
    context
}

/// Registers this frame's keyboard, IME and clipboard handling. Call during paint.
pub(super) fn register(
    element: &TerminalElement,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    window.set_key_context(key_context(element.searching));
    window.handle_input(
        &element.focus,
        ElementInputHandler::new(bounds, element.terminal.clone()),
        cx,
    );

    let terminal = element.terminal.clone();
    let blink = element.state.clone();
    window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            // Typing keeps the cursor on, even when the key is the application's.
            blink.blink_reset();
            keyboard::handle_key_down(&terminal, event, cx);
        }
    });
    let terminal = element.terminal.clone();
    window.on_action(TypeId::of::<input::Copy>(), move |_, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            clipboard::copy_selection(&terminal, cx);
        }
    });
    let terminal = element.terminal.clone();
    window.on_action(TypeId::of::<input::SelectAll>(), move |_, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            terminal.update(cx, |terminal, cx| terminal.select_all(cx));
        }
    });
    let terminal = element.terminal.clone();
    window.on_action(TypeId::of::<input::Clear>(), move |_, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            terminal.update(cx, |terminal, cx| terminal.clear(cx));
        }
    });
    for (action, scroll) in [
        (TypeId::of::<input::ScrollPageUp>(), TerminalScroll::PageUp),
        (
            TypeId::of::<input::ScrollPageDown>(),
            TerminalScroll::PageDown,
        ),
        (
            TypeId::of::<input::ScrollLineUp>(),
            TerminalScroll::Lines(1),
        ),
        (
            TypeId::of::<input::ScrollLineDown>(),
            TerminalScroll::Lines(-1),
        ),
    ] {
        let terminal = element.terminal.clone();
        window.on_action(action, move |_, phase, _, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // A full-screen program on the alternate screen has no history: it gets the key.
            if terminal
                .read(cx)
                .modes()
                .contains(TerminalModes::ALT_SCREEN)
            {
                cx.propagate();
                return;
            }
            terminal.update(cx, |terminal, cx| terminal.scroll(scroll, cx));
        });
    }
    let terminal = element.terminal.clone();
    let confirm = element.confirm.clone();
    window.on_action(TypeId::of::<input::Paste>(), move |_, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            clipboard::paste_clipboard(&terminal, confirm.as_ref(), window, cx);
        }
    });
}
