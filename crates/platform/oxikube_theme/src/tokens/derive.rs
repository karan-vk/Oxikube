//! Colours computed from other colours.

use super::{OxikubeColors, ThemeTokens};
use gpui::{Hsla, Rgba, hsla};

/// Fills the derived [`super::ThemeColors`] fields: `selection` (the first player's selection,
/// else the accent at 30 %) and `on_accent` (near-black on a light accent, white on a dark one),
/// and the derived [`super::TerminalColors`] fields: `cursor` (the first player's cursor, else the
/// accent) and `selection` (the interface selection).
pub(crate) fn derive_colors(tokens: &mut ThemeTokens) {
    let accent = tokens.colors.text_accent;
    let player = tokens.players.first();
    tokens.colors.selection = player.map_or_else(|| accent.opacity(0.3), |player| player.selection);
    tokens.colors.on_accent = on_accent_for(accent);
    tokens.terminal.cursor = player.map_or(accent, |player| player.cursor);
    tokens.terminal.selection = tokens.colors.selection;
}

/// Near-black or white, whichever has the higher WCAG contrast ratio against `accent`.
fn on_accent_for(accent: Hsla) -> Hsla {
    let ink = hsla(0.6, 0.1, 0.07, 1.0);
    let white = hsla(0.0, 0.0, 1.0, 1.0);
    let accent_luminance = luminance(accent);
    let contrast = |other: Hsla| {
        let (hi, lo) = (
            accent_luminance.max(luminance(other)),
            accent_luminance.min(luminance(other)),
        );
        (hi + 0.05) / (lo + 0.05)
    };
    if contrast(ink) >= contrast(white) {
        ink
    } else {
        white
    }
}

/// WCAG relative luminance of `color` (alpha ignored).
fn luminance(color: Hsla) -> f32 {
    let Rgba { r, g, b, .. } = Rgba::from(color);
    let linear = |c: f32| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The `oxikube` colours a theme gets when its file does not set them: status colours from the
/// theme's own status colours, cluster tabs cycling through its player colours (or accents, or
/// the terminal ANSI row when it has neither).
pub(crate) fn derive_oxikube(tokens: &ThemeTokens) -> OxikubeColors {
    let mut palette: Vec<Hsla> = tokens.players.iter().map(|p| p.cursor).collect();
    if palette.is_empty() {
        palette = tokens.accents.clone();
    }
    if palette.is_empty() {
        let ansi = tokens.terminal.ansi;
        palette = vec![
            ansi.blue,
            ansi.red,
            ansi.yellow,
            ansi.green,
            ansi.magenta,
            ansi.cyan,
        ];
    }
    let mut cluster_tabs = [tokens.colors.text_accent; super::CLUSTER_TAB_COLORS];
    for (slot, color) in cluster_tabs.iter_mut().zip(palette.iter().cycle()) {
        *slot = *color;
    }
    OxikubeColors {
        status_running: tokens.status.success.foreground,
        status_pending: tokens.status.warning.foreground,
        status_failed: tokens.status.error.foreground,
        status_succeeded: tokens.status.info.foreground,
        status_terminating: tokens.status.hidden.foreground,
        status_unknown: tokens.colors.text_muted,
        cluster_tabs,
    }
}
