//! Previewing a theme without choosing it (the theme picker's arrow keys).
//!
//! A preview overrides what [`crate::ActiveTheme`] shows but is not a setting: nothing is written, the
//! settings store and its watcher never hear of it, and [`refresh_active`] keeps honouring it, so
//! a settings reload or a system appearance flip while the picker is open does not clobber the
//! theme being looked at. [`clear_preview`] puts the theme the settings resolve to back.

use crate::global::refresh_active;
use crate::registry::ThemeRegistry;
use crate::tokens::ThemeTokens;
use gpui::{App, Global};
use std::sync::Arc;

/// The theme being previewed, held while a picker is open.
struct Preview(Arc<ThemeTokens>);

impl Global for Preview {}

/// Shows the registered theme called `name` as the active theme without selecting it. Returns
/// `false` (and changes nothing) when no theme has that name. Needs [`crate::init`] first.
pub fn preview_theme(cx: &mut App, name: &str) -> bool {
    let Some(tokens) = cx.global::<ThemeRegistry>().get(name) else {
        return false;
    };
    cx.set_global(Preview(tokens));
    refresh_active(cx);
    true
}

/// Ends a preview: the theme the `theme` setting resolves to is active again. A no-op when
/// nothing is previewed.
pub fn clear_preview(cx: &mut App) {
    if cx.has_global::<Preview>() {
        cx.remove_global::<Preview>();
        refresh_active(cx);
    }
}

/// Whether a preview is in effect.
pub fn is_previewing(cx: &App) -> bool {
    cx.has_global::<Preview>()
}

/// The previewed theme, for [`refresh_active`].
pub(crate) fn previewed(cx: &App) -> Option<Arc<ThemeTokens>> {
    cx.try_global::<Preview>().map(|preview| preview.0.clone())
}
