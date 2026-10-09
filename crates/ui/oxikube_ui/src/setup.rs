//! `init(cx)`: one call that makes the component library usable and themed.

use crate::theme_bridge::{reapply, set_tokens};
use crate::tokens::{Appearance, DefaultTokens, TokenSource, TokensGlobal};
use gpui::{App, Global};

/// Marks that [`init`] has run in this app.
pub(crate) struct Initialised;

impl Global for Initialised {}

/// Initialises `oxikube_ui`:
///
/// 1. initialises gpui-component (themes, key bindings, overlay plugins, tables, docks) and binds
///    the [`crate::code_view`] keys;
/// 2. resets the UI zoom to 100 %;
/// 3. installs the built-in tokens for the system appearance and projects them onto the
///    component library's theme (see [`crate::theme_bridge`]).
///
/// Idempotent: a second call is a no-op, so feature crates may each call it defensively. Tokens
/// set with [`set_tokens`] before `init` are kept and applied by it.
///
/// Not done here: registering the asset source. GPUI fixes it when the `Application` is built, so
/// the bin passes [`crate::Assets`] to `Application::with_assets`.
///
/// Once `oxikube_theme` lands, the bin follows `init` with [`set_token_source`] to swap the
/// built-in tokens for the active theme.
pub fn init(cx: &mut App) {
    if cx.has_global::<Initialised>() {
        return;
    }
    cx.set_global(Initialised);
    gpui_component::init(cx);
    crate::code_view::init(cx);
    crate::size::reset(cx);
    if cx.has_global::<TokensGlobal>() {
        // Tokens were chosen before init (e.g. restored from settings): apply those.
        reapply(cx);
    } else {
        let appearance = Appearance::from(cx.window_appearance());
        set_token_source(cx, &DefaultTokens, appearance);
    }
}

/// Applies the tokens `source` provides for `appearance`.
pub fn set_token_source(cx: &mut App, source: &dyn TokenSource, appearance: Appearance) {
    set_tokens(cx, source.tokens(appearance));
}
