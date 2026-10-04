//! The active tokens, as a GPUI global with a `cx.tokens()` accessor.

use super::tokens::{Colors, Tokens};
use gpui::{App, Global};

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
}

impl ActiveTokens for App {
    fn tokens(&self) -> Tokens {
        self.try_global::<TokensGlobal>()
            .map(|g| g.0)
            .unwrap_or_default()
    }
}
