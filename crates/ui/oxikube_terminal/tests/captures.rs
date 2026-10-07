//! Recorded full-screen programs through the grid (E09-S13): the bytes `vim`, `less` and `tmux`
//! (a split window with a status line, the layout `htop` and `k9s` paint) wrote to an 80x24
//! xterm-256color pty, committed under `tests/captures/` (`record.py` makes them), parsed by
//! [`TermGrid`] and asserted cell by cell: text, alternate screen, colours, bold, inverse video,
//! box-drawing borders and wide glyphs. The same bytes are painted by the screenshot suite
//! (`tests/screenshot_programs.rs`), so a parser regression fails here and a paint regression
//! there.
//!
//! The recordings never pass through a process: the test needs no `vim` and no cluster. A live
//! program in a pod (busybox `vi` and `top`) is the kind suite, `bins/oxikube/tests/kind_terminal`.

use oxikube_ports::TerminalSize;
use oxikube_terminal::TermGrid;
use oxikube_terminal::grid::{CellFlags, TermColor, TerminalModes, TerminalSnapshot};

const COLUMNS: u16 = 80;
const ROWS: u16 = 24;

/// The recording `name` (`tests/captures/<name>.vt`).
fn capture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/captures/{name}.vt", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn new_grid() -> TermGrid {
    TermGrid::new(TerminalSize::new(COLUMNS, ROWS), 1_000)
}

/// A grid that has played `name` back, in one write.
fn played(name: &str) -> TermGrid {
    let mut grid = new_grid();
    grid.advance(&capture(name), &mut Vec::new());
    grid
}

fn screen(snapshot: &TerminalSnapshot) -> Vec<String> {
    (0..snapshot.rows)
        .map(|row| snapshot.row_text(row))
        .collect()
}

#[test]
fn vim_draws_numbered_source_in_the_alternate_screen_with_syntax_colours() {
    let snapshot = played("vim").snapshot();
    assert!(snapshot.modes.contains(TerminalModes::ALT_SCREEN));
    let rows = screen(&snapshot);
    assert_eq!(rows[0], "  1 //! A small sample for the capture.");
    assert_eq!(
        rows[4],
        "  5 fn count(text: &str) -> HashMap<&str, usize> {"
    );
    assert_eq!(rows[9], " 10 }", "`dd` removed the line holding `seen`");
    assert_eq!(rows[14], " 15 }");
    for row in 15..23 {
        assert_eq!(rows[row], "~", "the empty rows past the file end: {row}");
    }

    // Syntax highlighting: the comment, the keyword and the identifier are three colours, none
    // of them the default; the `~` filler is its own colour again.
    let comment = snapshot.cell(0, 4).unwrap().fg;
    let keyword = snapshot.cell(4, 4).unwrap().fg;
    let identifier = snapshot.cell(4, 7).unwrap().fg;
    let filler = snapshot.cell(15, 0).unwrap().fg;
    let all = [comment, keyword, identifier, filler];
    for (index, color) in all.iter().enumerate() {
        assert_ne!(*color, TermColor::Foreground, "cell {index} is coloured");
        assert_eq!(
            all.iter().filter(|other| *other == color).count(),
            1,
            "{all:?}"
        );
    }
    // The cursor sits where `dd` left it: the line after the deleted one, at the first column of
    // the text (past the four-column number gutter).
    assert_eq!((snapshot.cursor.row, snapshot.cursor.column), (9, 4));
    assert!(snapshot.cursor.visible);
}

#[test]
fn less_scrolls_the_log_and_paints_the_prompt_in_inverse_video() {
    let snapshot = played("less").snapshot();
    assert!(snapshot.modes.contains(TerminalModes::ALT_SCREEN));
    let rows = screen(&snapshot);
    assert_eq!(rows[0], "INFO  line 18: pod web-18 is Pending -> Running");
    assert_eq!(rows[22], "INFO  line 40: pod web-40 is Pending -> Running");
    assert_eq!(rows[23], "(END)");

    // `\e[1;32mINFO` is bold green; the state words are yellow then green; the prompt is inverse.
    let level = snapshot.cell(0, 0).unwrap();
    assert_eq!(level.fg, TermColor::Indexed(2));
    assert!(level.flags.contains(CellFlags::BOLD));
    let text = snapshot.cell(0, 6).unwrap();
    assert_eq!(
        (text.fg, text.flags),
        (TermColor::Foreground, CellFlags::empty())
    );
    let state = rows[0].find("Pending").unwrap();
    assert_eq!(snapshot.cell(0, state).unwrap().fg, TermColor::Indexed(3));
    let arrow = rows[0].find("Running").unwrap();
    assert_eq!(snapshot.cell(0, arrow).unwrap().fg, TermColor::Indexed(2));
    assert!(
        snapshot
            .cell(23, 0)
            .unwrap()
            .flags
            .contains(CellFlags::INVERSE)
    );
}

#[test]
fn tmux_paints_two_panes_a_border_and_a_coloured_status_line() {
    let snapshot = played("tmux").snapshot();
    assert!(snapshot.modes.contains(TerminalModes::ALT_SCREEN));
    let rows = screen(&snapshot);

    // Left pane: an htop-like header, right pane: a log, in 40 columns each.
    assert!(
        rows[0].starts_with(" CPU[||||||||||||||      58%]"),
        "{:?}",
        rows[0]
    );
    assert!(
        rows[1].starts_with(" Mem[||||||||||      3.1G/8.0G]"),
        "{:?}",
        rows[1]
    );
    assert!(
        rows[2].contains("PID USER      CPU% COMMAND"),
        "{:?}",
        rows[2]
    );
    assert!(
        rows[4].contains("412 app       41.2 kubelet"),
        "{:?}",
        rows[4]
    );
    assert!(rows[0].contains("│INFO  web-1 Running"), "{:?}", rows[0]);
    assert!(rows[2].contains("│ERROR web-3 CrashLoop"), "{:?}", rows[2]);

    // The border is one column of box-drawing glyphs from the first row to the last pane row; tmux
    // draws the half beside the active pane in `pane-active-border-style` (green), the rest grey.
    let mut colours = Vec::new();
    for row in 0..23 {
        let border = snapshot.cell(row, 40).unwrap();
        assert_eq!(border.c, '│', "row {row}");
        if !colours.contains(&border.fg) {
            colours.push(border.fg);
        }
    }
    colours.sort_by_key(|colour| format!("{colour:?}"));
    assert_eq!(colours, [TermColor::Indexed(2), TermColor::Indexed(240)]);

    // The bars: bold green, then yellow, then red, as `printf` painted them.
    let bar = |column| snapshot.cell(0, column).unwrap();
    assert_eq!(bar(5).fg, TermColor::Indexed(2));
    assert!(bar(5).flags.contains(CellFlags::BOLD));
    assert_eq!(
        snapshot.cell(2, 1).unwrap().bg,
        TermColor::Indexed(6),
        "the cyan header row"
    );
    assert_eq!(snapshot.cell(2, 1).unwrap().fg, TermColor::Indexed(0));

    // The log levels: bold in their colour.
    let level = |row, expected: u8| {
        let cell = snapshot.cell(row, 41).unwrap();
        assert_eq!(cell.fg, TermColor::Indexed(expected), "row {row}");
        assert!(cell.flags.contains(CellFlags::BOLD), "row {row}");
    };
    level(0, 2);
    level(1, 3);
    level(2, 1);

    // A wide glyph in a 256-colour: two cells, the second a spacer.
    let wide = snapshot.cell(3, 47).unwrap();
    assert_eq!((wide.c, wide.fg), ('你', TermColor::Indexed(208)));
    assert!(wide.flags.contains(CellFlags::WIDE_CHAR));
    assert!(
        snapshot
            .cell(3, 48)
            .unwrap()
            .flags
            .contains(CellFlags::WIDE_CHAR_SPACER)
    );

    // The status line: white on blue across the whole width, session on the left, the load right.
    assert!(rows[23].starts_with("[demo] 0:"), "{:?}", rows[23]);
    assert!(rows[23].ends_with("load 0.42"), "{:?}", rows[23]);
    for column in 0..usize::from(COLUMNS) {
        let cell = snapshot.cell(23, column).unwrap();
        assert_eq!(cell.bg, TermColor::Indexed(4), "column {column}");
        assert_eq!(cell.fg, TermColor::Indexed(7), "column {column}");
    }
}

#[test]
fn the_screen_does_not_depend_on_how_the_bytes_arrive() {
    for name in ["vim", "less", "tmux"] {
        let bytes = capture(name);
        let whole = played(name).snapshot();
        // A pty delivers whatever the read returned: one byte at a time (every escape sequence
        // and UTF-8 sequence split at every point), then odd chunk sizes that straddle them.
        for chunk in [1, 2, 3, 7, 64, 1_000] {
            let mut grid = new_grid();
            let mut events = Vec::new();
            for piece in bytes.chunks(chunk) {
                grid.advance(piece, &mut events);
            }
            let snapshot = grid.snapshot();
            assert_eq!(snapshot.cells, whole.cells, "{name} in chunks of {chunk}");
            assert_eq!(snapshot.cursor, whole.cursor, "{name} in chunks of {chunk}");
            assert_eq!(snapshot.modes, whole.modes, "{name} in chunks of {chunk}");
        }
    }
}

#[test]
fn a_resize_keeps_the_alternate_screen_and_a_redraw_fills_the_new_size() {
    for name in ["vim", "less", "tmux"] {
        let mut grid = played(name);
        let before = grid.snapshot();
        // Growing keeps every cell; shrinking cuts them for good, so the last two come after the
        // check of the recorded corner.
        let sizes = [(120, 40), (80, 24), (40, 10), (2, 1)];
        for (index, (columns, rows)) in sizes.into_iter().enumerate() {
            if index == 2 {
                assert_eq!(
                    grid.snapshot().row_text(0),
                    before.row_text(0),
                    "{name}: back at 80x24"
                );
            }
            grid.resize(TerminalSize::new(columns, rows));
            let snapshot = grid.snapshot();
            assert_eq!(
                (snapshot.columns, snapshot.rows),
                (usize::from(columns), usize::from(rows))
            );
            assert!(
                snapshot.modes.contains(TerminalModes::ALT_SCREEN),
                "{name}: still the alternate screen"
            );
            assert!(
                snapshot.cursor.row < snapshot.rows && snapshot.cursor.column < snapshot.columns,
                "{name} {columns}x{rows}: the cursor {:?} is on the grid",
                snapshot.cursor
            );
            assert_eq!(
                snapshot.history_size, 0,
                "{name}: the alternate screen has no history"
            );
        }
    }

    // The program answers SIGWINCH by repainting at the new size: the last row of a 30-row grid
    // gets the status line the program draws there.
    let mut grid = played("less");
    grid.resize(TerminalSize::new(100, 30));
    grid.advance(
        b"\x1b[2J\x1b[H\x1b[30;1H\x1b[7m(END)\x1b[27m\x1b[1;1Hresized",
        &mut Vec::new(),
    );
    let snapshot = grid.snapshot();
    assert_eq!((snapshot.columns, snapshot.rows), (100, 30));
    assert_eq!(snapshot.row_text(0), "resized");
    assert_eq!(snapshot.row_text(29), "(END)");
    assert!(
        snapshot
            .cell(29, 0)
            .unwrap()
            .flags
            .contains(CellFlags::INVERSE)
    );
    assert_eq!(snapshot.row_text(23), "", "the old last row was cleared");
}
