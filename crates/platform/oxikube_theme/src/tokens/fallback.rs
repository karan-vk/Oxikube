//! The fallback tokens: the bundled One Dark / One Light, imported over a blank slate.

use super::{ThemeTokens, placeholder_color};
use crate::appearance::Appearance;
use std::sync::OnceLock;

static DARK: OnceLock<ThemeTokens> = OnceLock::new();
static LIGHT: OnceLock<ThemeTokens> = OnceLock::new();

/// See [`ThemeTokens::fallback`].
pub(super) fn fallback(appearance: Appearance) -> &'static ThemeTokens {
    let slot = if appearance.is_dark() { &DARK } else { &LIGHT };
    slot.get_or_init(|| build(appearance))
}

fn build(appearance: Appearance) -> ThemeTokens {
    let name = if appearance.is_dark() {
        "One Dark"
    } else {
        "One Light"
    };
    crate::import::import_bundled_over(name, ThemeTokens::placeholder(appearance))
        .map(fill_gaps)
        .unwrap_or_else(|| {
            // The embedded files are checked by tests; reaching this means a broken build.
            tracing::error!(
                name,
                "bundled fallback theme is missing; using a blank theme"
            );
            ThemeTokens::placeholder(appearance)
        })
}

/// Keys a bundled One file leaves unset (`null` or absent), given a sensible value.
fn fill_gaps(mut tokens: ThemeTokens) -> ThemeTokens {
    let blank = placeholder_color();
    let colors = tokens.colors;
    let conflict = tokens.status.conflict.background;
    let gaps = [
        (
            &mut tokens.colors.scrollbar_thumb_active,
            colors.scrollbar_thumb_hover,
        ),
        (
            &mut tokens.colors.panel_focused_border,
            colors.border_focused,
        ),
        (
            &mut tokens.colors.pane_focused_border,
            colors.border_focused,
        ),
        (&mut tokens.vcs.conflict_ours, conflict),
        (&mut tokens.vcs.conflict_theirs, conflict),
    ];
    for (slot, value) in gaps {
        if *slot == blank {
            *slot = value;
        }
    }
    tokens
}
