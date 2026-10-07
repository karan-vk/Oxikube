//! The table-driven key map: Zed `style` keys -> the [`ThemeTokens`] colour they set.
//!
//! One line per key, so supporting another Zed key is one line here (and a field in
//! `tokens::colors` when we do not have a slot for it yet). Keys not listed are ignored with a
//! debug log by the importer.

use crate::tokens::{OxikubeColors, ThemeTokens};
use gpui::Hsla;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Selects the colour a key sets inside a [`ThemeTokens`].
pub(crate) type ColorSlot = fn(&mut ThemeTokens) -> &mut Hsla;

/// Selects the colour an `oxikube` block key sets.
pub(crate) type OxikubeSlot = fn(&mut OxikubeColors) -> &mut Hsla;

macro_rules! slots {
    ($($key:literal => $($path:ident).+),+ $(,)?) => {
        &[$(($key, (|t: &mut ThemeTokens| &mut t.$($path).+) as ColorSlot)),+]
    };
}

/// Interface colours.
const UI: &[(&str, ColorSlot)] = slots! {
    "border" => colors.border,
    "border.variant" => colors.border_variant,
    "border.focused" => colors.border_focused,
    "border.selected" => colors.border_selected,
    "border.transparent" => colors.border_transparent,
    "border.disabled" => colors.border_disabled,
    "background" => colors.background,
    "surface.background" => colors.surface,
    "elevated_surface.background" => colors.elevated_surface,
    "element.background" => colors.element,
    "element.hover" => colors.element_hover,
    "element.active" => colors.element_active,
    "element.selected" => colors.element_selected,
    "element.disabled" => colors.element_disabled,
    "ghost_element.background" => colors.ghost_element,
    "ghost_element.hover" => colors.ghost_element_hover,
    "ghost_element.active" => colors.ghost_element_active,
    "ghost_element.selected" => colors.ghost_element_selected,
    "ghost_element.disabled" => colors.ghost_element_disabled,
    "drop_target.background" => colors.drop_target,
    "text" => colors.text,
    "text.muted" => colors.text_muted,
    "text.placeholder" => colors.text_placeholder,
    "text.disabled" => colors.text_disabled,
    "text.accent" => colors.text_accent,
    "icon" => colors.icon,
    "icon.muted" => colors.icon_muted,
    "icon.disabled" => colors.icon_disabled,
    "icon.placeholder" => colors.icon_placeholder,
    "icon.accent" => colors.icon_accent,
    "status_bar.background" => colors.status_bar,
    "title_bar.background" => colors.title_bar,
    "title_bar.inactive_background" => colors.title_bar_inactive,
    "toolbar.background" => colors.toolbar,
    "tab_bar.background" => colors.tab_bar,
    "tab.inactive_background" => colors.tab_inactive,
    "tab.active_background" => colors.tab_active,
    "panel.background" => colors.panel,
    "panel.focused_border" => colors.panel_focused_border,
    "pane.focused_border" => colors.pane_focused_border,
    "scrollbar.thumb.background" => colors.scrollbar_thumb,
    "scrollbar.thumb.hover_background" => colors.scrollbar_thumb_hover,
    "scrollbar.thumb.active_background" => colors.scrollbar_thumb_active,
    "scrollbar.thumb.border" => colors.scrollbar_thumb_border,
    "scrollbar.track.background" => colors.scrollbar_track,
    "scrollbar.track.border" => colors.scrollbar_track_border,
    "search.match_background" => colors.search_match,
    "search.active_match_background" => colors.search_active_match,
    "link_text.hover" => colors.link_text_hover,
};

/// Editor colours.
const EDITOR: &[(&str, ColorSlot)] = slots! {
    "editor.foreground" => editor.foreground,
    "editor.background" => editor.background,
    "editor.gutter.background" => editor.gutter_background,
    "editor.subheader.background" => editor.subheader_background,
    "editor.active_line.background" => editor.active_line_background,
    "editor.highlighted_line.background" => editor.highlighted_line_background,
    "editor.line_number" => editor.line_number,
    "editor.active_line_number" => editor.active_line_number,
    "editor.hover_line_number" => editor.hover_line_number,
    "editor.invisible" => editor.invisible,
    "editor.wrap_guide" => editor.wrap_guide,
    "editor.active_wrap_guide" => editor.active_wrap_guide,
    "editor.document_highlight.read_background" => editor.document_highlight_read_background,
    "editor.document_highlight.write_background" => editor.document_highlight_write_background,
};

/// Terminal colours.
const TERMINAL: &[(&str, ColorSlot)] = slots! {
    "terminal.background" => terminal.background,
    "terminal.foreground" => terminal.foreground,
    "terminal.bright_foreground" => terminal.bright_foreground,
    "terminal.dim_foreground" => terminal.dim_foreground,
    "terminal.ansi.black" => terminal.ansi.black,
    "terminal.ansi.red" => terminal.ansi.red,
    "terminal.ansi.green" => terminal.ansi.green,
    "terminal.ansi.yellow" => terminal.ansi.yellow,
    "terminal.ansi.blue" => terminal.ansi.blue,
    "terminal.ansi.magenta" => terminal.ansi.magenta,
    "terminal.ansi.cyan" => terminal.ansi.cyan,
    "terminal.ansi.white" => terminal.ansi.white,
    "terminal.ansi.bright_black" => terminal.bright.black,
    "terminal.ansi.bright_red" => terminal.bright.red,
    "terminal.ansi.bright_green" => terminal.bright.green,
    "terminal.ansi.bright_yellow" => terminal.bright.yellow,
    "terminal.ansi.bright_blue" => terminal.bright.blue,
    "terminal.ansi.bright_magenta" => terminal.bright.magenta,
    "terminal.ansi.bright_cyan" => terminal.bright.cyan,
    "terminal.ansi.bright_white" => terminal.bright.white,
    "terminal.ansi.dim_black" => terminal.dim.black,
    "terminal.ansi.dim_red" => terminal.dim.red,
    "terminal.ansi.dim_green" => terminal.dim.green,
    "terminal.ansi.dim_yellow" => terminal.dim.yellow,
    "terminal.ansi.dim_blue" => terminal.dim.blue,
    "terminal.ansi.dim_magenta" => terminal.dim.magenta,
    "terminal.ansi.dim_cyan" => terminal.dim.cyan,
    "terminal.ansi.dim_white" => terminal.dim.white,
};

/// Status colours (each with `.background` and `.border`) and VCS colours.
const STATUS: &[(&str, ColorSlot)] = slots! {
    "conflict" => status.conflict.foreground,
    "conflict.background" => status.conflict.background,
    "conflict.border" => status.conflict.border,
    "created" => status.created.foreground,
    "created.background" => status.created.background,
    "created.border" => status.created.border,
    "deleted" => status.deleted.foreground,
    "deleted.background" => status.deleted.background,
    "deleted.border" => status.deleted.border,
    "error" => status.error.foreground,
    "error.background" => status.error.background,
    "error.border" => status.error.border,
    "hidden" => status.hidden.foreground,
    "hidden.background" => status.hidden.background,
    "hidden.border" => status.hidden.border,
    "hint" => status.hint.foreground,
    "hint.background" => status.hint.background,
    "hint.border" => status.hint.border,
    "ignored" => status.ignored.foreground,
    "ignored.background" => status.ignored.background,
    "ignored.border" => status.ignored.border,
    "info" => status.info.foreground,
    "info.background" => status.info.background,
    "info.border" => status.info.border,
    "modified" => status.modified.foreground,
    "modified.background" => status.modified.background,
    "modified.border" => status.modified.border,
    "predictive" => status.predictive.foreground,
    "predictive.background" => status.predictive.background,
    "predictive.border" => status.predictive.border,
    "renamed" => status.renamed.foreground,
    "renamed.background" => status.renamed.background,
    "renamed.border" => status.renamed.border,
    "success" => status.success.foreground,
    "success.background" => status.success.background,
    "success.border" => status.success.border,
    "unreachable" => status.unreachable.foreground,
    "unreachable.background" => status.unreachable.background,
    "unreachable.border" => status.unreachable.border,
    "warning" => status.warning.foreground,
    "warning.background" => status.warning.background,
    "warning.border" => status.warning.border,
    "version_control.added" => vcs.added,
    "version_control.modified" => vcs.modified,
    "version_control.deleted" => vcs.deleted,
    "version_control.word_added" => vcs.word_added,
    "version_control.word_deleted" => vcs.word_deleted,
    "version_control.conflict_marker.ours" => vcs.conflict_ours,
    "version_control.conflict_marker.theirs" => vcs.conflict_theirs,
};

/// The `oxikube` block keys. `cluster.tab.<1..=8>` and `log.source.<1..=10>` are handled by index
/// in [`cluster_tab_index`] and [`log_source_index`].
pub(crate) const OXIKUBE: &[(&str, OxikubeSlot)] = &[
    ("status.running", |c| &mut c.status_running),
    ("status.pending", |c| &mut c.status_pending),
    ("status.failed", |c| &mut c.status_failed),
    ("status.succeeded", |c| &mut c.status_succeeded),
    ("status.terminating", |c| &mut c.status_terminating),
    ("status.unknown", |c| &mut c.status_unknown),
];

/// Prefix of the cluster tab palette keys (`cluster.tab.1` ..).
const CLUSTER_TAB_PREFIX: &str = "cluster.tab.";

/// The zero-based palette slot of an `oxikube` key like `cluster.tab.3`, when it is one.
pub(crate) fn cluster_tab_index(key: &str) -> Option<usize> {
    let n: usize = key.strip_prefix(CLUSTER_TAB_PREFIX)?.parse().ok()?;
    (1..=crate::tokens::CLUSTER_TAB_COLORS)
        .contains(&n)
        .then(|| n - 1)
}

/// Prefix of the log-source palette keys (`log.source.1` ..).
const LOG_SOURCE_PREFIX: &str = "log.source.";

/// The zero-based palette slot of an `oxikube` key like `log.source.3`, when it is one.
pub(crate) fn log_source_index(key: &str) -> Option<usize> {
    let n: usize = key.strip_prefix(LOG_SOURCE_PREFIX)?.parse().ok()?;
    (1..=crate::tokens::LOG_SOURCE_COLORS)
        .contains(&n)
        .then(|| n - 1)
}

static COLOR_KEYS: LazyLock<HashMap<&'static str, ColorSlot>> = LazyLock::new(|| {
    [UI, EDITOR, TERMINAL, STATUS]
        .into_iter()
        .flatten()
        .copied()
        .collect()
});

/// The slot a `style` key sets, if the key is mapped.
pub(crate) fn color_slot(key: &str) -> Option<ColorSlot> {
    COLOR_KEYS.get(key).copied()
}

/// Every mapped `style` key (for tests that check coverage).
#[cfg(test)]
pub(crate) fn mapped_keys() -> impl Iterator<Item = &'static str> {
    COLOR_KEYS.keys().copied()
}
