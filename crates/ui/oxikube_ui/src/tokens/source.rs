//! Where tokens come from.
//!
//! `oxikube_theme` (E05-S08) owns the real theme registry. Until it lands, `oxikube_ui` runs on
//! [`DefaultTokens`]. The dependency is deliberately an adapter trait: the theme crate (or the
//! bin that wires both) implements [`TokenSource`] and hands it to [`crate::set_token_source`],
//! so nothing in the views changes when the source flips.

use super::tokens::{Appearance, Tokens};

/// Produces [`Tokens`] for an appearance.
pub trait TokenSource {
    /// The tokens to use when the UI is in `appearance`.
    fn tokens(&self, appearance: Appearance) -> Tokens;
}

/// The built-in fallback source (see [`Tokens::default_for`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTokens;

impl TokenSource for DefaultTokens {
    fn tokens(&self, appearance: Appearance) -> Tokens {
        Tokens::default_for(appearance)
    }
}
