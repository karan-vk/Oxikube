//! Projects [`Tokens`] and `oxikube_theme` themes onto gpui-component's global `Theme`.
//!
//! This is the only place that writes the component library's theme. Views read [`Tokens`]
//! (through [`crate::ActiveTokens`]) and never gpui-component theme fields, so a library bump
//! that renames a field is fixed here and nowhere else.
//!
//! - this file: [`set_tokens`] and [`apply_tokens`], the per-component colour projection.
//! - `config`: [`set_theme`] and [`theme_config`], `ThemeTokens` -> the library's `ThemeConfig`
//!   (core colours, editor/syntax highlight) applied through its own theme mechanism.
//! - `follow`: [`follow_active_theme`], keeps the library in step with `oxikube_theme`.

mod config;
mod follow;

pub use config::{set_theme, theme_config};
pub use follow::follow_active_theme;

use crate::setup::Initialised;
use crate::size::UiScale;
use crate::tokens::{Appearance, Tokens, TokensGlobal};
use gpui::{App, Global, Hsla, hsla};
use gpui_component::{Theme, ThemeConfig, ThemeMode};
use std::rc::Rc;

/// The `ThemeConfig` the active tokens came from, when they came from a theme ([`set_theme`]).
struct ThemeConfigGlobal(Option<Rc<ThemeConfig>>);

impl Global for ThemeConfigGlobal {}

/// Installs `tokens` as the active set and re-themes the component library.
///
/// Before [`crate::init`] it only records the tokens; `init` applies them.
///
/// Switches gpui-component to the matching light/dark mode first (which loads that mode's
/// registered base theme), then overwrites the colours, radii and font sizes from `tokens`.
/// Refreshes all windows. Drops any theme config a previous [`set_theme`] recorded.
pub fn set_tokens(cx: &mut App, tokens: Tokens) {
    install(cx, tokens, None);
}

/// Records `tokens` (and the theme config they came from, if any) and applies them.
fn install(cx: &mut App, tokens: Tokens, config: Option<Rc<ThemeConfig>>) {
    cx.set_global(TokensGlobal(tokens));
    cx.set_global(ThemeConfigGlobal(config));
    reapply(cx);
}

/// Re-applies the stored tokens (after a zoom change, say).
pub(crate) fn reapply(cx: &mut App) {
    if !cx.has_global::<Initialised>() {
        return;
    }
    let Some(tokens) = cx.try_global::<TokensGlobal>().map(|g| g.0) else {
        return;
    };
    let config = cx
        .try_global::<ThemeConfigGlobal>()
        .and_then(|global| global.0.clone());
    let scale = UiScale::get(cx);
    let mode = match tokens.appearance {
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Light => ThemeMode::Light,
    };
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        if let Some(config) = &config {
            theme.apply_config(config);
        }
        apply_tokens(theme, &tokens, scale);
    });
}

/// Writes `tokens` into `theme`. Pure (no `App`), so it is unit-testable.
pub(crate) fn apply_tokens(theme: &mut Theme, tokens: &Tokens, scale: UiScale) {
    let k = scale.factor();
    theme.font_size = tokens.font.body * k;
    theme.mono_font_size = tokens.font.mono * k;
    theme.radius = tokens.radius.md * k;
    theme.radius_lg = tokens.radius.lg * k;

    let c = tokens.colors;
    let none = hsla(0., 0., 0., 0.);
    let t = &mut theme.colors;

    t.background = c.background;
    t.foreground = c.text;
    t.border = c.border;
    t.input = c.border;
    t.ring = c.border_focused;
    t.muted = c.element;
    t.muted_foreground = c.text_muted;
    t.popover = c.elevated_surface;
    t.popover_foreground = c.text;
    t.selection = c.selection;
    t.caret = c.text;
    t.overlay = hsla(
        0.,
        0.,
        0.,
        if tokens.appearance.is_dark() {
            0.55
        } else {
            0.35
        },
    );
    t.window_border = c.border;
    t.drag_border = c.accent;
    t.drop_target = c.accent.opacity(0.2);

    t.primary = c.accent;
    t.primary_hover = shade(c.accent, 0.05);
    t.primary_active = shade(c.accent, -0.05);
    t.primary_foreground = c.on_accent;
    t.secondary = c.element;
    t.secondary_hover = c.element_hover;
    t.secondary_active = c.element_active;
    t.secondary_foreground = c.text;
    t.accent = c.element_hover;
    t.accent_foreground = c.text;

    t.danger = c.error;
    t.danger_hover = shade(c.error, 0.05);
    t.danger_active = shade(c.error, -0.05);
    t.danger_foreground = c.on_accent;
    t.success = c.success;
    t.success_hover = shade(c.success, 0.05);
    t.success_active = shade(c.success, -0.05);
    t.success_foreground = c.on_accent;
    t.warning = c.warning;
    t.warning_hover = shade(c.warning, 0.05);
    t.warning_active = shade(c.warning, -0.05);
    t.warning_foreground = c.on_accent;
    t.info = c.info;
    t.info_hover = shade(c.info, 0.05);
    t.info_active = shade(c.info, -0.05);
    t.info_foreground = c.on_accent;
    t.link = c.accent;
    t.link_hover = shade(c.accent, 0.08);
    t.link_active = shade(c.accent, -0.05);

    t.button = c.element;
    t.button_hover = c.element_hover;
    t.button_active = c.element_active;
    t.button_foreground = c.text;
    t.button_primary = c.accent;
    t.button_primary_hover = shade(c.accent, 0.05);
    t.button_primary_active = shade(c.accent, -0.05);
    t.button_primary_foreground = c.on_accent;
    t.button_secondary = c.element;
    t.button_secondary_hover = c.element_hover;
    t.button_secondary_active = c.element_active;
    t.button_secondary_foreground = c.text;
    t.button_danger = c.error;
    t.button_danger_hover = shade(c.error, 0.05);
    t.button_danger_active = shade(c.error, -0.05);
    t.button_danger_foreground = c.on_accent;

    t.list = c.background;
    t.list_even = c.background;
    t.list_head = c.surface;
    t.list_hover = c.element_hover;
    t.list_active = c.element_selected;
    t.list_active_border = c.accent;

    t.table = c.background;
    t.table_even = c.surface.opacity(0.5);
    t.table_head = c.surface;
    t.table_head_foreground = c.text_muted;
    t.table_foot = c.surface;
    t.table_foot_foreground = c.text_muted;
    t.table_hover = c.element_hover;
    t.table_active = c.element_selected;
    t.table_active_border = c.accent;
    t.table_row_border = c.border_variant;

    t.sidebar = c.surface;
    t.sidebar_foreground = c.text;
    t.sidebar_border = c.border;
    t.sidebar_accent = c.element_hover;
    t.sidebar_accent_foreground = c.text;
    t.sidebar_primary = c.accent;
    t.sidebar_primary_foreground = c.on_accent;

    t.tab_bar = c.surface;
    t.tab_bar_segmented = c.element;
    t.tab = none;
    t.tab_active = c.background;
    t.tab_foreground = c.text_muted;
    t.tab_active_foreground = c.text;

    t.title_bar = c.surface;
    t.title_bar_border = c.border;
    t.status_bar = c.surface;
    t.status_bar_border = c.border;

    t.scrollbar = none;
    t.scrollbar_thumb = c.text_muted.opacity(0.4);
    t.scrollbar_thumb_hover = c.text_muted.opacity(0.7);

    t.skeleton = c.element;
    t.progress_bar = c.accent;
    t.slider_bar = c.accent;
    t.slider_thumb = c.on_accent;
    t.switch = c.element_active;
    t.switch_thumb = hsla(0., 0., 1., 1.);
    t.accordion = c.surface;
    t.group_box = c.surface;
    t.group_box_foreground = c.text;

    t.chart_1 = c.accent;
    t.chart_2 = c.success;
    t.chart_3 = c.warning;
    t.chart_4 = c.error;
    t.chart_5 = c.info;
    t.chart_grid = c.border_variant;
}

/// Lightness shifted by `delta` (clamped): hover/active variants derived from a base colour.
fn shade(color: Hsla, delta: f32) -> Hsla {
    Hsla {
        l: (color.l + delta).clamp(0.0, 1.0),
        ..color
    }
}

#[cfg(test)]
mod tests;
