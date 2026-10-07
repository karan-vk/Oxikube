//! alacritty → Oxikube conversions for snapshot types. The only place that maps alacritty's
//! colour, flag, mode and cursor enums, so a version bump that changes them breaks here.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{RenderableCursor, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape as AnsiCursorShape, NamedColor, Rgb};

use super::{CellFlags, CursorShape, TermColor, TermRgb, TerminalCursor, TerminalModes};

pub(super) fn rgb(rgb: Rgb) -> TermRgb {
    TermRgb {
        r: rgb.r,
        g: rgb.g,
        b: rgb.b,
    }
}

pub(super) fn color(color: Color) -> TermColor {
    match color {
        Color::Spec(spec) => TermColor::Rgb(rgb(spec)),
        Color::Indexed(index) => TermColor::Indexed(index),
        Color::Named(named) => named_color(named),
    }
}

fn named_color(named: NamedColor) -> TermColor {
    match named {
        NamedColor::Foreground => TermColor::Foreground,
        NamedColor::Background => TermColor::Background,
        NamedColor::Cursor => TermColor::Cursor,
        NamedColor::BrightForeground => TermColor::BrightForeground,
        NamedColor::DimForeground => TermColor::DimForeground,
        NamedColor::DimBlack => TermColor::Dim(0),
        NamedColor::DimRed => TermColor::Dim(1),
        NamedColor::DimGreen => TermColor::Dim(2),
        NamedColor::DimYellow => TermColor::Dim(3),
        NamedColor::DimBlue => TermColor::Dim(4),
        NamedColor::DimMagenta => TermColor::Dim(5),
        NamedColor::DimCyan => TermColor::Dim(6),
        NamedColor::DimWhite => TermColor::Dim(7),
        // Black ..= BrightWhite are palette entries 0..16 (the enum's discriminants).
        ansi => TermColor::Indexed(ansi as u8),
    }
}

const FLAG_MAP: [(Flags, CellFlags); 15] = [
    (Flags::BOLD, CellFlags::BOLD),
    (Flags::ITALIC, CellFlags::ITALIC),
    (Flags::UNDERLINE, CellFlags::UNDERLINE),
    (Flags::DOUBLE_UNDERLINE, CellFlags::DOUBLE_UNDERLINE),
    (Flags::UNDERCURL, CellFlags::UNDERCURL),
    (Flags::DOTTED_UNDERLINE, CellFlags::DOTTED_UNDERLINE),
    (Flags::DASHED_UNDERLINE, CellFlags::DASHED_UNDERLINE),
    (Flags::INVERSE, CellFlags::INVERSE),
    (Flags::DIM, CellFlags::DIM),
    (Flags::HIDDEN, CellFlags::HIDDEN),
    (Flags::STRIKEOUT, CellFlags::STRIKEOUT),
    (Flags::WIDE_CHAR, CellFlags::WIDE_CHAR),
    (Flags::WIDE_CHAR_SPACER, CellFlags::WIDE_CHAR_SPACER),
    (
        Flags::LEADING_WIDE_CHAR_SPACER,
        CellFlags::LEADING_WIDE_CHAR_SPACER,
    ),
    (Flags::WRAPLINE, CellFlags::WRAPLINE),
];

pub(super) fn flags(flags: Flags) -> CellFlags {
    if flags.is_empty() {
        return CellFlags::empty();
    }
    FLAG_MAP
        .iter()
        .filter(|(theirs, _)| flags.contains(*theirs))
        .fold(CellFlags::empty(), |acc, (_, ours)| acc | *ours)
}

const MODE_MAP: [(TermMode, TerminalModes); 14] = [
    (TermMode::SHOW_CURSOR, TerminalModes::SHOW_CURSOR),
    (TermMode::APP_CURSOR, TerminalModes::APP_CURSOR),
    (TermMode::APP_KEYPAD, TerminalModes::APP_KEYPAD),
    (TermMode::BRACKETED_PASTE, TerminalModes::BRACKETED_PASTE),
    (TermMode::FOCUS_IN_OUT, TerminalModes::FOCUS_IN_OUT),
    (TermMode::ALT_SCREEN, TerminalModes::ALT_SCREEN),
    (TermMode::LINE_WRAP, TerminalModes::LINE_WRAP),
    (TermMode::ALTERNATE_SCROLL, TerminalModes::ALTERNATE_SCROLL),
    (
        TermMode::MOUSE_REPORT_CLICK,
        TerminalModes::MOUSE_REPORT_CLICK,
    ),
    (TermMode::MOUSE_DRAG, TerminalModes::MOUSE_DRAG),
    (TermMode::MOUSE_MOTION, TerminalModes::MOUSE_MOTION),
    (TermMode::SGR_MOUSE, TerminalModes::SGR_MOUSE),
    (TermMode::UTF8_MOUSE, TerminalModes::UTF8_MOUSE),
    (
        TermMode::LINE_FEED_NEW_LINE,
        TerminalModes::LINE_FEED_NEW_LINE,
    ),
];

pub(super) fn modes(mode: TermMode) -> TerminalModes {
    let mut modes = MODE_MAP
        .iter()
        .filter(|(theirs, _)| mode.contains(*theirs))
        .fold(TerminalModes::empty(), |acc, (_, ours)| acc | *ours);
    if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        modes |= TerminalModes::KITTY_KEYBOARD;
    }
    modes
}

/// The cursor in viewport coordinates; hidden when the process hid it or the view is scrolled
/// away from it.
pub(super) fn cursor(
    cursor: RenderableCursor,
    display_offset: usize,
    rows: usize,
) -> TerminalCursor {
    let row = cursor.point.line.0 + display_offset as i32;
    let in_view = (0..rows as i32).contains(&row);
    let (shape, shown) = match cursor.shape {
        AnsiCursorShape::Block => (CursorShape::Block, true),
        AnsiCursorShape::Underline => (CursorShape::Underline, true),
        AnsiCursorShape::Beam => (CursorShape::Beam, true),
        AnsiCursorShape::HollowBlock => (CursorShape::HollowBlock, true),
        AnsiCursorShape::Hidden => (CursorShape::Block, false),
    };
    TerminalCursor {
        row: row.max(0) as usize,
        column: cursor.point.column.0,
        shape,
        visible: shown && in_view,
    }
}
