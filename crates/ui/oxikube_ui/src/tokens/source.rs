//! Where tokens come from.
//!
//! `oxikube_theme` (E05-S08) owns the theme registry and the active theme. `oxikube_ui` runs on
//! [`DefaultTokens`] until the bin calls [`crate::follow_active_theme`], which applies the active
//! theme (and every later change of it) through [`crate::set_theme`]; nothing in the views
//! changes when the source flips. [`TokenSource`] stays for sources that only know an appearance.

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
