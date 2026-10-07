//! The colour groups of a theme: interface chrome, editor, terminal, status and VCS.

use super::macros::color_struct;
use gpui::Hsla;

color_struct! {
    /// Interface colours: surfaces, borders, element states, text and icons.
    ///
    /// Names follow Zed's theme keys (`element.hover` -> `element_hover`), so a Zed theme maps
    /// one to one. Two fields are derived rather than read from a file: [`ThemeColors::selection`]
    /// and [`ThemeColors::on_accent`].
    ThemeColors {
        border: "Strong border: inputs, dividers between panels.",
        border_variant: "Quiet border: table row separators.",
        border_focused: "Border of the focused control.",
        border_selected: "Border of a selected element.",
        border_transparent: "Fully transparent border (keeps layout stable).",
        border_disabled: "Border of a disabled control.",
        background: "Window chrome background (title bar, tab bar).",
        surface: "Panels and docks.",
        elevated_surface: "Popovers, menus and dialogs.",
        element: "Resting interactive element (button, input).",
        element_hover: "Element under the pointer.",
        element_active: "Element being pressed.",
        element_selected: "Selected element.",
        element_disabled: "Disabled element.",
        ghost_element: "Resting element that has no fill until interacted with.",
        ghost_element_hover: "Ghost element under the pointer.",
        ghost_element_active: "Ghost element being pressed.",
        ghost_element_selected: "Selected ghost element.",
        ghost_element_disabled: "Disabled ghost element.",
        drop_target: "Highlight of a valid drop target.",
        text: "Primary text.",
        text_muted: "Secondary text: captions, column headers.",
        text_placeholder: "Placeholder text in inputs.",
        text_disabled: "Disabled text.",
        text_accent: "Accent text: links, the primary action colour.",
        icon: "Primary icon.",
        icon_muted: "Secondary icon.",
        icon_disabled: "Disabled icon.",
        icon_placeholder: "Placeholder icon.",
        icon_accent: "Accent icon.",
        status_bar: "Status bar background.",
        title_bar: "Title bar background.",
        title_bar_inactive: "Title bar background in an unfocused window.",
        toolbar: "Toolbar background.",
        tab_bar: "Tab strip background.",
        tab_inactive: "Inactive tab background.",
        tab_active: "Active tab background.",
        panel: "Dock panel background.",
        panel_focused_border: "Border of the focused panel.",
        pane_focused_border: "Border of the focused pane.",
        scrollbar_thumb: "Scrollbar thumb.",
        scrollbar_thumb_hover: "Scrollbar thumb under the pointer.",
        scrollbar_thumb_active: "Scrollbar thumb being dragged.",
        scrollbar_thumb_border: "Scrollbar thumb border.",
        scrollbar_track: "Scrollbar track.",
        scrollbar_track_border: "Scrollbar track border.",
        search_match: "Background of a search match.",
        search_active_match: "Background of the current search match.",
        link_text_hover: "Link text under the pointer.",
        selection: "Text selection background (derived: the first player's selection colour).",
        on_accent: "Text drawn on an accent fill (derived: black or white for contrast).",
    }
}

color_struct! {
    /// Editor (YAML / JSON / log text) colours.
    EditorColors {
        foreground: "Default text.",
        background: "Editor background.",
        gutter_background: "Line-number gutter background.",
        subheader_background: "Sticky header background.",
        active_line_background: "Background of the line with the cursor.",
        highlighted_line_background: "Background of a highlighted line.",
        line_number: "Line number.",
        active_line_number: "Line number of the active line.",
        hover_line_number: "Line number under the pointer.",
        invisible: "Whitespace markers.",
        wrap_guide: "Soft-wrap guide.",
        active_wrap_guide: "Soft-wrap guide of the active line.",
        document_highlight_read_background: "Background of a read occurrence.",
        document_highlight_write_background: "Background of a write occurrence.",
    }
}

color_struct! {
    /// The eight ANSI colours of one terminal palette row.
    AnsiColors {
        black: "ANSI black.",
        red: "ANSI red.",
        green: "ANSI green.",
        yellow: "ANSI yellow.",
        blue: "ANSI blue.",
        magenta: "ANSI magenta.",
        cyan: "ANSI cyan.",
        white: "ANSI white.",
    }
}

/// Terminal colours: base, foreground variants, cursor and selection, and the normal/bright/dim
/// ANSI rows (the 16 colours plus their dim row; the terminal element derives the rest of the
/// 256-colour palette from them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalColors {
    /// Terminal background.
    pub background: Hsla,
    /// Default foreground.
    pub foreground: Hsla,
    /// Foreground of bold text.
    pub bright_foreground: Hsla,
    /// Foreground of faint text.
    pub dim_foreground: Hsla,
    /// The cursor (derived: the first player's cursor colour, else the accent text colour). Zed
    /// theme files have no terminal cursor key.
    pub cursor: Hsla,
    /// The selection highlight drawn over cells (derived: the interface selection colour).
    pub selection: Hsla,
    /// Normal ANSI colours.
    pub ansi: AnsiColors,
    /// Bright ANSI colours.
    pub bright: AnsiColors,
    /// Dim ANSI colours.
    pub dim: AnsiColors,
}

impl TerminalColors {
    pub(crate) fn splat(color: Hsla) -> Self {
        Self {
            background: color,
            foreground: color,
            bright_foreground: color,
            dim_foreground: color,
            cursor: color,
            selection: color,
            ansi: AnsiColors::splat(color),
            bright: AnsiColors::splat(color),
            dim: AnsiColors::splat(color),
        }
    }
}

/// One status colour: the glyph/text colour with its tinted background and border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColor {
    /// Text or glyph colour.
    pub foreground: Hsla,
    /// Tinted background (a toast, a table cell).
    pub background: Hsla,
    /// Border.
    pub border: Hsla,
}

impl StatusColor {
    pub(crate) fn splat(color: Hsla) -> Self {
        Self {
            foreground: color,
            background: color,
            border: color,
        }
    }
}

/// Zed's status colours: diagnostics-style states and VCS-style change states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColors {
    /// Conflict.
    pub conflict: StatusColor,
    /// Created.
    pub created: StatusColor,
    /// Deleted.
    pub deleted: StatusColor,
    /// Error.
    pub error: StatusColor,
    /// Hidden.
    pub hidden: StatusColor,
    /// Hint.
    pub hint: StatusColor,
    /// Ignored.
    pub ignored: StatusColor,
    /// Information.
    pub info: StatusColor,
    /// Modified.
    pub modified: StatusColor,
    /// Predictive (suggested) content.
    pub predictive: StatusColor,
    /// Renamed.
    pub renamed: StatusColor,
    /// Success.
    pub success: StatusColor,
    /// Unreachable.
    pub unreachable: StatusColor,
    /// Warning.
    pub warning: StatusColor,
}

impl StatusColors {
    pub(crate) fn splat(color: Hsla) -> Self {
        let c = StatusColor::splat(color);
        Self {
            conflict: c,
            created: c,
            deleted: c,
            error: c,
            hidden: c,
            hint: c,
            ignored: c,
            info: c,
            modified: c,
            predictive: c,
            renamed: c,
            success: c,
            unreachable: c,
            warning: c,
        }
    }
}

color_struct! {
    /// Version-control (diff) colours, used by YAML diffs and the Helm/Argo drift views.
    VcsColors {
        added: "Added line.",
        modified: "Modified line.",
        deleted: "Deleted line.",
        word_added: "Added word within a line.",
        word_deleted: "Deleted word within a line.",
        conflict_ours: "Conflict marker, our side.",
        conflict_theirs: "Conflict marker, their side.",
    }
}
