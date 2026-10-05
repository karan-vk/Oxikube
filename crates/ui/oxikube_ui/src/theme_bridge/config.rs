//! `ThemeTokens` -> gpui-component's [`ThemeConfig`], and applying it.
//!
//! The config carries what the library derives its other colours from (core colours, the
//! `base.*` palette from the terminal ANSI row), the editor/syntax `highlight` section (which
//! takes Zed `style` data as is) and the theme's name and mode. [`set_theme`] applies it with
//! [`gpui_component::Theme::apply_config`] (which also records it as the library's light or dark
//! theme), then [`super::apply_tokens`] pins Oxikube's per-component colours on top.
//!
//! The `highlight` section is fed to the library as plain JSON in Zed's `style` format. The
//! library reads it for editor and syntax colours once its `tree-sitter` feature is on (the
//! editor epic turns it on); without it the section type is a stub that ignores the data.

use super::install;
use crate::tokens::Tokens;
use gpui::{App, Hsla};
use gpui_component::ThemeConfig;
use oxikube_theme::color::to_hex;
use oxikube_theme::tokens::{FontStyle, SyntaxStyle};
use oxikube_theme::{Appearance, ThemeTokens};
use serde_json::{Map, Value, json};
use std::rc::Rc;

/// Makes `theme` the active theme: its colours become the active [`Tokens`] and the component
/// library is re-themed from [`theme_config`].
///
/// Before [`crate::init`] it only records the theme; `init` applies it. Refreshes all windows.
pub fn set_theme(cx: &mut App, theme: &ThemeTokens) {
    install(cx, Tokens::from(theme), Some(Rc::new(theme_config(theme))));
}

/// The gpui-component theme configuration for `theme`.
///
/// Pure (no `App`): unit-testable, and the one place the library's config schema is spoken.
pub fn theme_config(theme: &ThemeTokens) -> ThemeConfig {
    let value = json!({
        "name": theme.name,
        "mode": if theme.appearance == Appearance::Dark { "dark" } else { "light" },
        "colors": colors(theme),
        "highlight": highlight(theme),
    });
    // A config that does not deserialize is a bug in the maps below (the tests check every
    // bundled theme); the theme still applies through `apply_tokens`, so degrade to a blank one.
    serde_json::from_value(value).unwrap_or_default()
}

fn hex(color: Hsla) -> String {
    to_hex(color)
}

/// The core colours, from the same [`Tokens`] the views read, so config and overlay agree.
fn colors(theme: &ThemeTokens) -> Value {
    let c = Tokens::from(theme).colors;
    let ansi = theme.terminal.ansi;
    let bright = theme.terminal.bright;
    let pairs: &[(&str, Hsla)] = &[
        ("background", c.background),
        ("foreground", c.text),
        ("border", c.border),
        ("input.border", c.border),
        ("ring", c.border_focused),
        ("caret", c.text),
        ("selection.background", c.selection),
        ("muted.background", c.element),
        ("muted.foreground", c.text_muted),
        ("popover.background", c.elevated_surface),
        ("popover.foreground", c.text),
        ("primary.background", c.accent),
        ("primary.foreground", c.on_accent),
        ("secondary.background", c.element),
        ("secondary.hover.background", c.element_hover),
        ("secondary.active.background", c.element_active),
        ("secondary.foreground", c.text),
        ("accent.background", c.element_hover),
        ("accent.foreground", c.text),
        ("danger.background", c.error),
        ("danger.foreground", c.on_accent),
        ("success.background", c.success),
        ("success.foreground", c.on_accent),
        ("warning.background", c.warning),
        ("warning.foreground", c.on_accent),
        ("info.background", c.info),
        ("info.foreground", c.on_accent),
        ("link", c.accent),
        ("list.background", c.background),
        ("list.head.background", c.surface),
        ("list.hover.background", c.element_hover),
        ("list.active.background", c.element_selected),
        ("table.background", c.background),
        ("table.head.background", c.surface),
        ("table.head.foreground", c.text_muted),
        ("table.hover.background", c.element_hover),
        ("table.active.background", c.element_selected),
        ("table.row.border", c.border_variant),
        ("sidebar.background", c.surface),
        ("sidebar.foreground", c.text),
        ("sidebar.border", c.border),
        ("tab_bar.background", c.surface),
        ("tab.active.background", c.background),
        ("title_bar.background", c.surface),
        ("title_bar.border", c.border),
        ("status_bar.background", c.surface),
        ("status_bar.border", c.border),
        ("scrollbar.thumb.background", theme.colors.scrollbar_thumb),
        (
            "scrollbar.thumb.hover.background",
            theme.colors.scrollbar_thumb_hover,
        ),
        ("base.red", ansi.red),
        ("base.red.light", bright.red),
        ("base.green", ansi.green),
        ("base.green.light", bright.green),
        ("base.yellow", ansi.yellow),
        ("base.yellow.light", bright.yellow),
        ("base.blue", ansi.blue),
        ("base.blue.light", bright.blue),
        ("base.magenta", ansi.magenta),
        ("base.magenta.light", bright.magenta),
        ("base.cyan", ansi.cyan),
        ("base.cyan.light", bright.cyan),
    ];
    Value::Object(
        pairs
            .iter()
            .map(|(key, color)| ((*key).to_owned(), Value::String(hex(*color))))
            .collect(),
    )
}

/// The `highlight` section: Zed's `style` keys the library reads (editor colours, status colours
/// and `syntax`).
pub(super) fn highlight(theme: &ThemeTokens) -> Value {
    let e = theme.editor;
    let mut style = Map::new();
    let mut put = |key: &str, color: Hsla| {
        style.insert(key.to_owned(), Value::String(hex(color)));
    };
    put("editor.background", e.background);
    put("editor.foreground", e.foreground);
    put("editor.active_line.background", e.active_line_background);
    put("editor.line_number", e.line_number);
    put("editor.active_line_number", e.active_line_number);
    put("editor.invisible", e.invisible);
    put("editor.gutter.background", e.gutter_background);
    for (name, status) in [
        ("error", theme.status.error),
        ("warning", theme.status.warning),
        ("info", theme.status.info),
        ("success", theme.status.success),
        ("hint", theme.status.hint),
    ] {
        put(name, status.foreground);
        put(&format!("{name}.background"), status.background);
        put(&format!("{name}.border"), status.border);
    }
    let syntax: Map<String, Value> = theme
        .syntax
        .styles
        .iter()
        .map(|(name, style)| (name.clone(), syntax_style(style)))
        .collect();
    style.insert("syntax".to_owned(), Value::Object(syntax));
    Value::Object(style)
}

fn syntax_style(style: &SyntaxStyle) -> Value {
    let mut entry = Map::new();
    if let Some(color) = style.color {
        entry.insert("color".into(), Value::String(hex(color)));
    }
    // gpui-component has no oblique; italic is the nearest.
    match style.font_style {
        Some(FontStyle::Italic | FontStyle::Oblique) => {
            entry.insert("font_style".into(), json!("italic"));
        }
        Some(FontStyle::Normal) => {
            entry.insert("font_style".into(), json!("normal"));
        }
        None => {}
    }
    if let Some(weight) = style.font_weight {
        // The library accepts the nine CSS weights only.
        let nearest = ((weight / 100.0).round().clamp(1.0, 9.0) as u16) * 100;
        entry.insert("font_weight".into(), json!(nearest));
    }
    Value::Object(entry)
}
