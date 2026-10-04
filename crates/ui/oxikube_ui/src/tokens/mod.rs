//! Design tokens: colours, spacing, radii, font sizes, and the global that holds the active set.
//!
//! - `tokens`: the value types ([`Tokens`], [`Colors`], [`Spacing`], [`Radius`], [`FontSizes`]).
//! - `source`: the [`TokenSource`] adapter trait `oxikube_theme` will implement.
//! - `active`: the global plus the [`ActiveTokens`] accessor (`cx.tokens()`, `cx.colors()`).

mod active;
mod source;
#[allow(clippy::module_inception)]
mod tokens;

pub use active::ActiveTokens;
pub(crate) use active::TokensGlobal;
pub use source::{DefaultTokens, TokenSource};
pub use tokens::{Appearance, Colors, FontSizes, Radius, Spacing, Tokens};
