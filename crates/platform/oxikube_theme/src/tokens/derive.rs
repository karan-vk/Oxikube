//! Colours computed from other colours.

use super::contrast::{luminance, with_min_contrast};
use super::{OxikubeColors, ThemeTokens};
use gpui::{Hsla, hsla};

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

/// WCAG AA for normal-size text.
const MIN_TEXT_CONTRAST: f32 = 4.5;

/// The `oxikube` colours a theme gets when its file does not set them: status colours from the
/// theme's own status colours, cluster tabs cycling through its player colours (or accents, or
/// the terminal ANSI row when it has neither), and the log-source palette from the terminal's
/// ANSI colours (moved lighter or darker where one is below AA on the log row fills).
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
    let (ansi, bright) = (tokens.terminal.ansi, tokens.terminal.bright);
    let log_sources = [
        ansi.blue,
        ansi.green,
        ansi.yellow,
        ansi.magenta,
        ansi.cyan,
        bright.blue,
        bright.green,
        bright.yellow,
        bright.magenta,
        bright.cyan,
    ];
    // A pod's name is text on the log rows (plain, matched and current-match fills), and ANSI
    // colours are tuned for a terminal, not for AA on those (One Light's yellow is 1.9:1).
    let backgrounds = [
        tokens.editor.background,
        tokens.colors.element,
        tokens.colors.element_selected,
    ];
    let log_sources = log_sources.map(|c| with_min_contrast(c, &backgrounds, MIN_TEXT_CONTRAST));
    OxikubeColors {
        status_running: tokens.status.success.foreground,
        status_pending: tokens.status.warning.foreground,
        status_failed: tokens.status.error.foreground,
        status_succeeded: tokens.status.info.foreground,
        status_terminating: tokens.status.hidden.foreground,
        status_unknown: tokens.colors.text_muted,
        cluster_tabs,
        log_sources,
    }
}
