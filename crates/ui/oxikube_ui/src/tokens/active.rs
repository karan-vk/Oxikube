//! The active tokens, as a GPUI global with a `cx.tokens()` accessor.

use super::tokens::{Colors, Tokens};
use gpui::{App, Global, SharedString};

/// The [`Tokens`] currently applied. Set by [`crate::set_tokens`]; read with
/// [`ActiveTokens::tokens`].
pub(crate) struct TokensGlobal(pub(crate) Tokens);

impl Global for TokensGlobal {}

/// `cx.tokens()` / `cx.colors()` for any context that derefs to [`App`].
pub trait ActiveTokens {
    /// The active tokens. Falls back to the built-in dark tokens before [`crate::init`] has run,
    /// so a view can always render.
    fn tokens(&self) -> Tokens;

    /// Shorthand for `self.tokens().colors`.
    fn colors(&self) -> Colors {
        self.tokens().colors
    }

    /// The monospace font family of the active theme (log lines, terminal, code). A generic
    /// `monospace` before [`crate::init`] has run.
    fn mono_font_family(&self) -> SharedString;
}

impl ActiveTokens for App {
    fn tokens(&self) -> Tokens {
        self.try_global::<TokensGlobal>()
            .map(|g| g.0)
            .unwrap_or_default()
    }

    fn mono_font_family(&self) -> SharedString {
        self.try_global::<gpui_component::Theme>().map_or_else(
            || "monospace".into(),
            |theme| theme.mono_font_family.clone(),
        )
    }
}
