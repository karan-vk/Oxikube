//! Oxikube binary. Wires adapters into the app and mounts the UI.
//!
//! Until E05 lands this is a placeholder window proving the GPUI stack
//! (gpui-pre + gpui-component) resolves and renders on macOS and Linux.
//! Init order will follow Zed's `main.rs` pattern: logging → settings → keymap →
//! theme → AppState → each crate's `init(cx)` → workspace restore.

use gpui::{App, Context, Window, WindowOptions, div, prelude::*, rgb};

struct Placeholder;

impl Render for Placeholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e2127))
            .text_color(rgb(0xd7dae0))
            .text_xl()
            .child("Oxikube — workspace skeleton. See docs/ROADMAP.md.")
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Placeholder))
            .expect("open main window");
        cx.activate(true);
    });
}
