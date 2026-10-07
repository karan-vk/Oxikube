//! Per-frame upkeep of the element's state during prepaint: dropping a stale link hover and
//! rebuilding the palette when the theme or the process's colour overrides changed.

use std::sync::Arc;

use oxikube_theme::ThemeTokens;

use super::palette::TerminalPalette;
use super::{Inner, hash};
use crate::grid::TermRgb;

/// The palette and what it was built from.
pub(super) struct PaletteMemo {
    pub(super) theme: Arc<ThemeTokens>,
    pub(super) overrides: Vec<(usize, TermRgb)>,
    pub(super) palette: TerminalPalette,
}

/// Drops the hovered link when a row it is on shows something else (output, scrolling): the next
/// pointer move finds it again. Detection itself never runs per frame.
pub(super) fn forget_stale_hover(inner: &mut Inner) {
    if inner.hovered.is_none() {
        return;
    }
    let snapshot = &inner.snapshot;
    let changed = inner
        .hover_rows
        .iter()
        .any(|&(row, hash)| row >= snapshot.rows || hash::row_hash(snapshot, row) != hash);
    if changed {
        inner.hovered = None;
        inner.hover_cell = None;
        inner.hover_rows.clear();
    }
}

/// Rebuilds the palette when the theme's terminal colours or the snapshot's overrides changed.
pub(super) fn refresh_palette(inner: &mut Inner, theme: Arc<ThemeTokens>) {
    let overrides = &inner.snapshot.color_overrides;
    let fresh = inner.palette.as_ref().is_some_and(|memo| {
        (Arc::ptr_eq(&memo.theme, &theme) || memo.theme.terminal == theme.terminal)
            && memo.overrides == *overrides
    });
    if !fresh {
        let palette = TerminalPalette::new(&theme.terminal).with_overrides(overrides);
        inner.palette = Some(PaletteMemo {
            theme,
            overrides: overrides.clone(),
            palette,
        });
    }
}
