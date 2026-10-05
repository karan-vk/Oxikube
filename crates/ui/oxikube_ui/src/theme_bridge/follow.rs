//! Keeps the component library in step with `oxikube_theme`'s active theme.

use super::set_theme;
use gpui::{App, Subscription};
use oxikube_theme::ActiveTheme;

/// Applies the active theme now and again whenever it changes (a settings edit, the system
/// light/dark flip, a hot-reloaded theme file). Returns the observer's subscription; the bin
/// keeps it (or `.detach()`es it) for the life of the app.
///
/// Call after [`crate::init`] and `oxikube_theme::init`. [`ActiveTheme`] only changes when the
/// resolved tokens differ, so this re-themes (and refreshes windows) once per real change.
pub fn follow_active_theme(cx: &mut App) -> Subscription {
    apply_active(cx);
    cx.observe_global::<ActiveTheme>(apply_active)
}

fn apply_active(cx: &mut App) {
    let theme = ActiveTheme::get(cx);
    set_theme(cx, &theme);
}
