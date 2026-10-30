//! A theme switch recolours a live terminal on the next frame (E22-S10).

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_terminal::element::TerminalPalette;
use oxikube_terminal::grid::TermColor;
use oxikube_theme::{ActiveTheme, Appearance, ThemeTokens};

use super::{FONT_SIZE, harness};

fn switch_to(h: &mut super::Harness, appearance: Appearance) -> Arc<ThemeTokens> {
    let theme = Arc::new(ThemeTokens::fallback(appearance).clone());
    let set = theme.clone();
    h.window
        .update(move |_, cx| cx.set_global(ActiveTheme(set)));
    theme
}

#[gpui::test]
fn the_palette_follows_the_active_theme_live(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("\x1b[31mred\x1b[0m");
    let dark = ThemeTokens::fallback(Appearance::Dark);
    assert_eq!(
        h.state.palette().expect("painted"),
        TerminalPalette::from_theme(dark)
    );

    let light = switch_to(&mut h, Appearance::Light);
    h.frame();
    let palette = h.state.palette().expect("painted");
    assert_eq!(palette, TerminalPalette::from_theme(&light));
    assert_eq!(
        palette.resolve(TermColor::Indexed(1)),
        light.terminal.ansi.red
    );
    assert_eq!(palette.background(), light.terminal.background);
    assert_eq!(palette.cursor(), light.terminal.cursor);
    assert_eq!(palette.selection(), light.terminal.selection);
    assert_ne!(palette.background(), dark.terminal.background);
}

#[gpui::test]
fn a_theme_switch_notifies_the_terminal_once(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    let notified = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = notified.clone();
    let _subscription = h.window.update(|_, cx| {
        let terminal = h.terminal.clone();
        cx.observe(&terminal, move |_, _| counter.set(counter.get() + 1))
    });
    switch_to(&mut h, Appearance::Light);
    h.frame();
    assert_eq!(notified.get(), 1, "one notify per theme change");
}
