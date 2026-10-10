//! The display rows of a text: where each row starts and ends, made off the UI thread.
//!
//! A row is at most `cols` columns of one line. With soft wrap, `cols` is what fits the view and a
//! long line breaks after its last space that fits (or mid-word when it has none); without it,
//! `cols` is [`MAX_ROW_COLS`] and only a line longer than that is cut, so no row ever asks the
//! text system to shape more than a bounded run of glyphs. The font is monospace, so a column is
//! one advance; a wide (East Asian, emoji) character counts two.
//!
//! The map is a plain `Vec` (24 bytes a row), sized from a newline count before it is filled.

/// The most columns one row holds when lines are not wrapped: a longer line continues on the
/// next row (each row is shaped as one run, so this bounds a frame's shaping).
pub const MAX_ROW_COLS: usize = 1_000;

/// One display row: bytes `start..end` of the text (no newline), part of line `line` (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// First byte of the row.
    pub start: usize,
    /// End of the row (exclusive), before any `\r\n` or `\n`.
    pub end: usize,
    /// The line the row belongs to.
    pub line: usize,
}

/// The fewest columns a wrapped row gets, however narrow the view.
pub(super) const MIN_WRAP_COLS: usize = 16;

/// The rows of a text at a given width.
#[derive(Debug, Clone, Default)]
pub struct RowMap {
    cols: usize,
    /// The columns of the line-number gutter (0 without one).
    gutter_cols: usize,
    rows: Vec<Row>,
    /// The row with the most columns (the horizontal extent without wrap).
    widest: usize,
}

impl RowMap {
    /// The rows of `text`. `width` is the view's width in columns, gutter included: with it, long
    /// lines wrap at what is left after the gutter, breaking after a space when one fits; without
    /// it (`None`), lines are only cut at [`MAX_ROW_COLS`]. `gutter` reserves room for line
    /// numbers (the widest number and a column either side).
    pub fn build(text: &str, width: Option<usize>, gutter: bool) -> Self {
        let lines = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let gutter_cols = if gutter { digits(lines) + 2 } else { 0 };
        let cols = width.map_or(MAX_ROW_COLS, |width| {
            width.saturating_sub(gutter_cols).max(MIN_WRAP_COLS)
        });
        let word_wrap = width.is_some();
        let mut map = RowMap {
            cols,
            gutter_cols,
            rows: Vec::with_capacity(lines),
            widest: 0,
        };
        let mut widest_cols = 0;
        let mut base = 0;
        let body = text.strip_suffix('\n').unwrap_or(text);
        for (line, raw) in body.split('\n').enumerate() {
            let content = raw.strip_suffix('\r').unwrap_or(raw);
            let (width, row) = if content.len() <= cols && content.is_ascii() {
                map.rows.push(Row {
                    start: base,
                    end: base + content.len(),
                    line,
                });
                (content.len(), map.rows.len() - 1)
            } else {
                map.split_line(content, base, line, word_wrap)
            };
            if width > widest_cols {
                widest_cols = width;
                map.widest = row;
            }
            base += raw.len() + 1;
        }
        map
    }

    /// Pushes the rows of one long (or non-ASCII) line; returns its widest row's columns and
    /// index (the first of equally wide rows, which is a full row rather than a shorter tail).
    fn split_line(
        &mut self,
        line: &str,
        base: usize,
        line_ix: usize,
        word_wrap: bool,
    ) -> (usize, usize) {
        let cols = self.cols;
        let mut start = 0;
        let mut width = 0;
        // The widest row so far: its columns and its index in `rows`.
        let mut widest = (0, self.rows.len());
        // The byte just after the last space of the row, and the row's width up to it.
        let mut space: Option<(usize, usize)> = None;
        for (at, ch) in line.char_indices() {
            let w = char_cols(ch);
            if width + w > cols && at > start {
                let (cut, row_cols) = match space {
                    Some((after, before)) if word_wrap && after > start && after <= at => {
                        width -= before;
                        (after, before)
                    }
                    _ => (at, std::mem::take(&mut width)),
                };
                if row_cols > widest.0 {
                    widest = (row_cols, self.rows.len());
                }
                self.rows.push(Row {
                    start: base + start,
                    end: base + cut,
                    line: line_ix,
                });
                start = cut;
                space = None;
            }
            width += w;
            if word_wrap && ch == ' ' {
                space = Some((at + 1, width));
            }
        }
        if width > widest.0 {
            widest = (width, self.rows.len());
        }
        self.rows.push(Row {
            start: base + start,
            end: base + line.len(),
            line: line_ix,
        });
        widest
    }

    /// The columns a row holds at most.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The columns of the line-number gutter (0 without one).
    pub fn gutter_cols(&self) -> usize {
        self.gutter_cols
    }

    /// How many rows there are (an empty text has one, empty).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows (never, for a built map).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row `ix`.
    pub fn row(&self, ix: usize) -> Option<Row> {
        self.rows.get(ix).copied()
    }

    /// Whether row `ix` starts its line (it gets the line number).
    pub fn starts_line(&self, ix: usize) -> bool {
        ix == 0 || self.rows.get(ix - 1).map(|r| r.line) != self.rows.get(ix).map(|r| r.line)
    }

    /// How many lines the text has.
    pub fn lines(&self) -> usize {
        self.rows.last().map_or(0, |row| row.line + 1)
    }

    /// The row with the most columns.
    pub fn widest(&self) -> usize {
        self.widest
    }

    /// The row that holds byte `byte` of the text (the last row for a byte past the end; a
    /// newline belongs to the row it ends).
    pub fn row_of_byte(&self, byte: usize) -> usize {
        self.rows
            .partition_point(|row| row.start <= byte)
            .saturating_sub(1)
    }

    /// The first row of line `line` (the last row for a line past the end).
    pub fn first_row_of_line(&self, line: usize) -> usize {
        self.rows
            .partition_point(|row| row.line < line)
            .min(self.rows.len().saturating_sub(1))
    }
}

/// How many decimal digits `n` has.
fn digits(n: usize) -> usize {
    n.checked_ilog10().map_or(1, |log| log as usize + 1)
}

/// The columns `ch` takes in a monospace font: two for wide East Asian characters and emoji,
/// one otherwise.
pub(super) fn char_cols(ch: char) -> usize {
    let c = u32::from(ch);
    if c < 0x1100 {
        return 1;
    }
    let wide = matches!(c,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

/// The byte offset in `row_text` of the character boundary nearest column boundary `col` (clamped
/// to the end of the row).
pub(super) fn offset_at_col(row_text: &str, col: usize) -> usize {
    let mut width = 0;
    for (at, ch) in row_text.char_indices() {
        if width >= col {
            return at;
        }
        let w = char_cols(ch);
        if width + w > col {
            // `col` falls inside a wide character: the nearer of its two edges.
            return if (col - width) * 2 >= w {
                at + ch.len_utf8()
            } else {
                at
            };
        }
        width += w;
    }
    row_text.len()
}
