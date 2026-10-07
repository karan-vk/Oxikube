//! `ThemeTokens`: the neutral, GPUI-independent description of one theme.
//!
//! Only `gpui::Hsla` is used from GPUI (a plain colour value), so tokens are `Send + Sync` and
//! cheap to share in an `Arc`. Zed's own `ThemeColors` (about 159 fields) is not copied: the
//! groups here hold the keys Oxikube draws with, and `crate::import` maps a Zed theme onto them
//! through an extendable table.
//!
//! - `colors`: interface, editor, terminal, status and VCS colour groups.
//! - `syntax`: syntax highlight styles and player colours.
//! - `oxikube`: the Kubernetes status colours and cluster-tab palette.
//! - `derive`: the colours computed from others (selection, text on accent, `oxikube` defaults).
//! - `fallback`: the tokens a theme falls back to for keys it does not set.

mod colors;
mod derive;
mod fallback;
mod macros;
mod oxikube;
mod syntax;

use crate::appearance::Appearance;
use gpui::{Hsla, hsla};

pub use colors::{
    AnsiColors, EditorColors, StatusColor, StatusColors, TerminalColors, ThemeColors, VcsColors,
};
pub(crate) use derive::{derive_colors, derive_oxikube};
pub use oxikube::{CLUSTER_TAB_COLORS, LOG_SOURCE_COLORS, OxikubeColors};
pub use syntax::{FontStyle, PlayerColor, SyntaxStyle, SyntaxTheme};

/// One theme, resolved: every colour Oxikube draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeTokens {
    /// Display name, e.g. `One Dark`.
    pub name: String,
    /// Whether this is a light or a dark theme.
    pub appearance: Appearance,
    /// Interface colours.
    pub colors: ThemeColors,
    /// Editor colours.
    pub editor: EditorColors,
    /// Terminal colours.
    pub terminal: TerminalColors,
    /// Status colours (error, warning, created, ...).
    pub status: StatusColors,
    /// Version-control (diff) colours.
    pub vcs: VcsColors,
    /// Player colours; index 0 is the local user.
    pub players: Vec<PlayerColor>,
    /// Accent colours for categorical use (tags, series).
    pub accents: Vec<Hsla>,
    /// Syntax highlight styles.
    pub syntax: SyntaxTheme,
    /// Kubernetes status colours and the cluster-tab palette.
    pub oxikube: OxikubeColors,
}

impl ThemeTokens {
    /// The tokens a theme falls back to for keys it does not set: One Dark or One Light.
    ///
    /// Parsed from the bundled files once per process.
    pub fn fallback(appearance: Appearance) -> &'static ThemeTokens {
        fallback::fallback(appearance)
    }

    /// Every colour the same unmistakable placeholder: the blank slate the bundled fallbacks are
    /// imported over (a leftover placeholder would show a key the table does not map).
    pub(crate) fn placeholder(appearance: Appearance) -> Self {
        let color = placeholder_color();
        Self {
            name: String::new(),
            appearance,
            colors: ThemeColors::splat(color),
            editor: EditorColors::splat(color),
            terminal: TerminalColors::splat(color),
            status: StatusColors::splat(color),
            vcs: VcsColors::splat(color),
            players: Vec::new(),
            accents: Vec::new(),
            syntax: SyntaxTheme::default(),
            oxikube: OxikubeColors::splat(color),
        }
    }
}

/// The colour [`ThemeTokens::placeholder`] fills with (magenta at an odd alpha).
pub(crate) fn placeholder_color() -> Hsla {
    hsla(0.8333, 1.0, 0.5, 0.123)
}
