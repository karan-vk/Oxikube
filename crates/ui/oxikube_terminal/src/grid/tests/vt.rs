//! Colours, attributes, cursor movement, title, modes and replies.

use super::super::{CellFlags, CursorShape, GridEvent, TermColor, TermRgb, TerminalModes};
use super::{feed, grid, screen};

#[test]
fn plain_text_and_newlines_land_in_the_grid() {
    let mut grid = grid(10, 3, 100);
    feed(&mut grid, "hello\r\nworld");
    let snap = grid.snapshot();
    assert_eq!((snap.columns, snap.rows), (10, 3));
    assert_eq!(screen(&snap), ["hello", "world", ""]);
    assert_eq!((snap.cursor.row, snap.cursor.column), (1, 5));
    assert!(snap.cursor.visible);
}

#[test]
fn sgr_colours_and_attributes() {
    let mut grid = grid(20, 2, 0);
    // Red bold, 256-colour fg on palette bg, 24-bit fg, reset, then bright green.
    feed(
        &mut grid,
        "\x1b[1;31mA\x1b[38;5;200;48;5;17mB\x1b[0;38;2;10;20;30mC\x1b[0m\x1b[92mD\x1b[0mE",
    );
    let snap = grid.snapshot();
    let a = snap.cell(0, 0).unwrap();
    assert_eq!(a.c, 'A');
    assert_eq!(a.fg, TermColor::Indexed(1));
    assert!(a.flags.contains(CellFlags::BOLD));
    let b = snap.cell(0, 1).unwrap();
    assert_eq!(
        (b.fg, b.bg),
        (TermColor::Indexed(200), TermColor::Indexed(17))
    );
    assert!(b.flags.contains(CellFlags::BOLD), "SGR 38/48 keep bold");
    let c = snap.cell(0, 2).unwrap();
    assert_eq!(
        c.fg,
        TermColor::Rgb(TermRgb {
            r: 10,
            g: 20,
            b: 30
        })
    );
    assert!(!c.flags.contains(CellFlags::BOLD), "SGR 0 resets");
    assert_eq!(snap.cell(0, 3).unwrap().fg, TermColor::Indexed(10));
    let e = snap.cell(0, 4).unwrap();
    assert_eq!((e.fg, e.bg), (TermColor::Foreground, TermColor::Background));
}

#[test]
fn italic_underline_inverse_strikeout_dim() {
    let mut grid = grid(10, 1, 0);
    feed(
        &mut grid,
        "\x1b[3mi\x1b[0;4mu\x1b[0;7mv\x1b[0;9ms\x1b[0;2md\x1b[0;4:3mc",
    );
    let snap = grid.snapshot();
    let flags: Vec<_> = (0..6).map(|col| snap.cell(0, col).unwrap().flags).collect();
    assert_eq!(
        flags,
        [
            CellFlags::ITALIC,
            CellFlags::UNDERLINE,
            CellFlags::INVERSE,
            CellFlags::STRIKEOUT,
            CellFlags::DIM,
            CellFlags::UNDERCURL,
        ]
    );
}

#[test]
fn cursor_movement_sequences() {
    let mut grid = grid(10, 5, 0);
    // CUP to row 3 col 4 (1-based), write, then relative moves.
    feed(&mut grid, "\x1b[3;4HX\x1b[2AY\x1b[1;1H\x1b[2CZ");
    let snap = grid.snapshot();
    assert_eq!(snap.row_text(2), "   X");
    assert_eq!(snap.row_text(0), "  Z Y");
    assert_eq!((snap.cursor.row, snap.cursor.column), (0, 3));
}

#[test]
fn erase_display_and_line() {
    let mut grid = grid(10, 3, 0);
    feed(&mut grid, "aaaa\r\nbbbb\r\ncccc");
    feed(&mut grid, "\x1b[2;3H\x1b[K");
    assert_eq!(screen(&grid.snapshot()), ["aaaa", "bb", "cccc"]);
    feed(&mut grid, "\x1b[2J");
    assert_eq!(screen(&grid.snapshot()), ["", "", ""]);
}

#[test]
fn title_is_set_and_reset() {
    let mut grid = grid(10, 2, 0);
    let events = feed(&mut grid, "\x1b]0;my shell\x07");
    assert_eq!(events, [GridEvent::TitleChanged(Some("my shell".into()))]);
    assert_eq!(grid.snapshot().title.as_deref(), Some("my shell"));
    assert_eq!(grid.title().map(|t| &**t), Some("my shell"));
}

#[test]
fn bell_and_clipboard_copy_are_events() {
    let mut grid = grid(10, 2, 0);
    assert_eq!(feed(&mut grid, "\x07"), [GridEvent::Bell]);
    // OSC 52 copy of base64("hi").
    assert_eq!(
        feed(&mut grid, "\x1b]52;c;aGk=\x07"),
        [GridEvent::ClipboardStore("hi".into())]
    );
    // OSC 52 paste request: refused, nothing is asked of the outside world.
    assert!(feed(&mut grid, "\x1b]52;c;?\x07").is_empty());
}

#[test]
fn queries_are_answered_with_replies() {
    let mut grid = grid(10, 4, 0);
    // Cursor position report after moving to row 2 col 5.
    let events = feed(&mut grid, "\x1b[2;5H\x1b[6n");
    assert_eq!(events, [GridEvent::Reply("\x1b[2;5R".into())]);
    // Primary device attributes.
    let events = feed(&mut grid, "\x1b[c");
    assert!(matches!(&events[..], [GridEvent::Reply(bytes)] if bytes.starts_with(b"\x1b[?")));
    // Text area size in characters.
    let events = feed(&mut grid, "\x1b[18t");
    assert_eq!(events, [GridEvent::Reply("\x1b[8;4;10t".into())]);
}

#[test]
fn colour_queries_go_out_unless_the_process_set_the_colour() {
    let mut grid = grid(10, 2, 0);
    let events = feed(&mut grid, "\x1b]11;?\x07");
    let [GridEvent::ColorRequest(request)] = &events[..] else {
        panic!("expected a colour request, got {events:?}");
    };
    assert_eq!(request.index(), 257);
    let reply = request.reply(TermRgb {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    });
    assert_eq!(&reply[..], b"\x1b]11;rgb:1212/3434/5656\x07");

    // Palette entry 1 redefined by the process: answered from the grid, and in the snapshot.
    feed(&mut grid, "\x1b]4;1;rgb:ff/00/00\x07");
    let events = feed(&mut grid, "\x1b]4;1;?\x07");
    assert!(
        matches!(&events[..], [GridEvent::Reply(bytes)] if bytes.starts_with(b"\x1b]4;1;rgb:ffff/0000/0000"))
    );
    assert_eq!(
        grid.snapshot().color_overrides,
        [(
            1,
            TermRgb {
                r: 0xff,
                g: 0,
                b: 0
            }
        )]
    );
}

#[test]
fn modes_follow_the_process() {
    let mut grid = grid(10, 2, 0);
    let defaults = grid.modes();
    assert!(defaults.contains(TerminalModes::SHOW_CURSOR | TerminalModes::LINE_WRAP));
    assert!(!defaults.intersects(TerminalModes::APP_CURSOR | TerminalModes::MOUSE_MODE));

    feed(
        &mut grid,
        "\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?25l",
    );
    let modes = grid.snapshot().modes;
    assert!(modes.contains(
        TerminalModes::APP_CURSOR
            | TerminalModes::BRACKETED_PASTE
            | TerminalModes::MOUSE_REPORT_CLICK
            | TerminalModes::SGR_MOUSE
            | TerminalModes::FOCUS_IN_OUT
    ));
    assert!(!modes.contains(TerminalModes::SHOW_CURSOR));
    assert!(!grid.snapshot().cursor.visible, "DECTCEM hides the cursor");

    feed(&mut grid, "\x1b[?1l\x1b[?2004l\x1b[?25h");
    let modes = grid.modes();
    assert!(!modes.intersects(TerminalModes::APP_CURSOR | TerminalModes::BRACKETED_PASTE));
    assert!(grid.snapshot().cursor.visible);
}

#[test]
fn cursor_shape_follows_decscusr() {
    let mut grid = grid(10, 2, 0);
    assert_eq!(grid.snapshot().cursor.shape, CursorShape::Block);
    feed(&mut grid, "\x1b[5 q");
    assert_eq!(grid.snapshot().cursor.shape, CursorShape::Beam);
    feed(&mut grid, "\x1b[3 q");
    assert_eq!(grid.snapshot().cursor.shape, CursorShape::Underline);
}

#[test]
fn input_split_mid_sequence_parses_the_same() {
    let mut whole = grid(10, 2, 0);
    feed(&mut whole, "\x1b[1;32mok\x1b[0m");
    let mut split = grid(10, 2, 0);
    for byte in "\x1b[1;32mok\x1b[0m".bytes() {
        feed(&mut split, [byte]);
    }
    assert_eq!(whole.snapshot().cells, split.snapshot().cells);
}

#[test]
fn utf8_split_across_chunks() {
    let mut grid = grid(10, 1, 0);
    let bytes = "é✓".as_bytes();
    feed(&mut grid, &bytes[..1]);
    feed(&mut grid, &bytes[1..4]);
    feed(&mut grid, &bytes[4..]);
    assert_eq!(grid.snapshot().row_text(0), "é✓");
}
