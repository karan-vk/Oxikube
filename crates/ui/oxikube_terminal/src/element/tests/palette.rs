//! The palette resolves through the theme's terminal tokens.

use gpui::{Hsla, Rgba};
use oxikube_theme::{Appearance, ThemeTokens};

use super::super::palette::TerminalPalette;
use crate::grid::{CellFlags, SnapshotCell, TermColor, TermRgb};

fn theme() -> &'static ThemeTokens {
    ThemeTokens::fallback(Appearance::Dark)
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    })
}

#[test]
fn the_sixteen_colours_and_the_defaults_come_from_the_theme() {
    let t = &theme().terminal;
    let palette = TerminalPalette::new(t);
    assert_eq!(palette.resolve(TermColor::Indexed(0)), t.ansi.black);
    assert_eq!(palette.resolve(TermColor::Indexed(1)), t.ansi.red);
    assert_eq!(palette.resolve(TermColor::Indexed(7)), t.ansi.white);
    assert_eq!(palette.resolve(TermColor::Indexed(9)), t.bright.red);
    assert_eq!(palette.resolve(TermColor::Indexed(15)), t.bright.white);
    assert_eq!(palette.resolve(TermColor::Dim(4)), t.dim.blue);
    assert_eq!(palette.resolve(TermColor::Foreground), t.foreground);
    assert_eq!(palette.resolve(TermColor::Background), t.background);
    assert_eq!(
        palette.resolve(TermColor::BrightForeground),
        t.bright_foreground
    );
    assert_eq!(palette.resolve(TermColor::DimForeground), t.dim_foreground);
    assert_eq!(palette.cursor(), t.cursor);
    assert_eq!(palette.selection(), t.selection);
}

#[test]
fn the_cube_the_greys_and_truecolour_follow_xterm() {
    let palette = TerminalPalette::new(&theme().terminal);
    assert_eq!(palette.resolve(TermColor::Indexed(16)), rgb(0, 0, 0));
    assert_eq!(palette.resolve(TermColor::Indexed(196)), rgb(255, 0, 0));
    assert_eq!(palette.resolve(TermColor::Indexed(67)), rgb(95, 135, 175));
    assert_eq!(palette.resolve(TermColor::Indexed(231)), rgb(255, 255, 255));
    assert_eq!(palette.resolve(TermColor::Indexed(232)), rgb(8, 8, 8));
    assert_eq!(palette.resolve(TermColor::Indexed(255)), rgb(238, 238, 238));
    let true_colour = TermColor::Rgb(TermRgb { r: 1, g: 2, b: 3 });
    assert_eq!(palette.resolve(true_colour), rgb(1, 2, 3));
}

#[test]
fn process_overrides_win_over_the_theme() {
    let red = TermRgb { r: 200, g: 0, b: 0 };
    let black = TermRgb { r: 0, g: 0, b: 0 };
    let palette = TerminalPalette::new(&theme().terminal).with_overrides(&[(1, red), (257, black)]);
    assert_eq!(palette.resolve(TermColor::Indexed(1)), rgb(200, 0, 0));
    assert_eq!(palette.background(), rgb(0, 0, 0));
    assert_eq!(
        palette.resolve(TermColor::Indexed(2)),
        theme().terminal.ansi.green
    );
}

fn cell(fg: TermColor, bg: TermColor, flags: CellFlags) -> SnapshotCell {
    SnapshotCell {
        c: 'x',
        fg,
        bg,
        flags,
    }
}

#[test]
fn attributes_change_the_colours() {
    let t = &theme().terminal;
    let palette = TerminalPalette::new(t);
    let plain = cell(
        TermColor::Foreground,
        TermColor::Background,
        CellFlags::empty(),
    );
    assert_eq!(
        palette.cell_colors(&plain),
        (t.foreground, None),
        "default bg is not painted"
    );

    let bold = cell(
        TermColor::Foreground,
        TermColor::Background,
        CellFlags::BOLD,
    );
    assert_eq!(palette.cell_colors(&bold).0, t.bright_foreground);

    let dim = cell(TermColor::Foreground, TermColor::Background, CellFlags::DIM);
    assert_eq!(palette.cell_colors(&dim).0, t.dim_foreground);
    let dim_red = cell(TermColor::Indexed(1), TermColor::Background, CellFlags::DIM);
    assert_eq!(palette.cell_colors(&dim_red).0, t.dim.red);

    let inverse = cell(
        TermColor::Indexed(2),
        TermColor::Background,
        CellFlags::INVERSE,
    );
    assert_eq!(
        palette.cell_colors(&inverse),
        (t.background, Some(t.ansi.green))
    );

    let hidden = cell(
        TermColor::Indexed(1),
        TermColor::Indexed(4),
        CellFlags::HIDDEN,
    );
    assert_eq!(
        palette.cell_colors(&hidden),
        (t.ansi.blue, Some(t.ansi.blue)),
        "hidden text is drawn in its background colour"
    );
}
