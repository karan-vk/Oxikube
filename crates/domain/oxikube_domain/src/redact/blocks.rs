//! The `secret-data-block` scanner: multi-line `data:` / `stringData:` blocks.
//!
//! A manifest often arrives escaped: inside a JSON log line, or in the `Debug` form of a `&str`
//! field, its line breaks are `\n` escapes (`\\n` when escaped twice). Headers are therefore
//! found at every break, real or escaped. The header's own break then fixes the block's
//! *escape level*, and inside the block only a break at that level or a shallower one ends a
//! line: a deeper escape is a `\n` inside a quoted value (`config: "a\nb"`), not a new line.
//! When the block sits inside a string (level 1 and up), the quote that closes that string
//! ends the block too, so the framing after it (`","target":…`) is never read as entries,
//! and a header on the string's first line is indented from the string's opening quote, not
//! from the outer line that frames it (see [`header_indent`]).

use super::scrubber::{DATA_HEADER, DATA_PAIR};
use super::values::{Cut, replace_values};
use std::borrow::Cow;

/// An open block: the header's indent and the escape level of its line breaks.
#[derive(Debug, Clone, Copy)]
struct Block {
    indent: usize,
    level: u32,
}

/// One line: its content, the break that ended it (`""` at the end of the text or at a closing
/// quote), and whether a closing quote ended it.
#[derive(Debug, PartialEq, Eq)]
struct Line<'a> {
    content: &'a str,
    ending: &'a str,
    closed: bool,
}

impl Line<'_> {
    fn len(&self) -> usize {
        self.content.len() + self.ending.len()
    }
}

/// Redacts the entries indented under every `data:` / `stringData:` header line.
pub(super) fn scrub_data_blocks(input: &str) -> Cow<'_, str> {
    if !lines(input).any(|(content, _)| DATA_HEADER.is_match(content)) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut block: Option<Block> = None;
    while !rest.is_empty() {
        let line = next_line(rest, block.map(|b| b.level));
        if let Some(open) = block {
            let blank = line.content.trim().is_empty();
            if !blank && indent(line.content) <= open.indent {
                // Back at the header's indent: the block is over. Scan this line again as
                // ordinary text, splitting at every break, since it may open another block.
                block = None;
                continue;
            }
            if blank {
                out.push_str(line.content);
            } else {
                out.push_str(&replace_values(
                    &DATA_PAIR,
                    line.content,
                    Cut::FramingOrKey,
                    |_| false,
                ));
            }
            if line.closed {
                block = None;
            }
        } else if !line.ending.is_empty() && DATA_HEADER.is_match(line.content) {
            let level = escape_level(line.ending);
            block = Some(Block {
                indent: header_indent(line.content, level),
                level,
            });
            out.push_str(line.content);
        } else {
            out.push_str(line.content);
        }
        out.push_str(line.ending);
        rest = &rest[line.len()..];
    }
    if out == input {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(out)
    }
}

fn indent(content: &str) -> usize {
    content.len() - content.trim_start().len()
}

/// The indent a header at escape `level` gives its block: an entry must be deeper to belong.
///
/// At level 0 it is the header line's own indent. Inside a string (level 1 and up) the header
/// may sit on the string's first line, after the framing that opened it: pretty JSON
/// (`    "manifest": "data:`) or `{:#?}` Debug (`    manifest: "data:`). The entries are
/// indented relative to the string's text, not to that outer line, so the indent is also
/// measured from just after the string's opening quote, and the smaller of the two is kept:
/// reading a line too many as an entry over-redacts, one too few leaks. The block still ends
/// at the quote that closes the string.
fn header_indent(content: &str, level: u32) -> usize {
    let line = indent(content);
    if level == 0 {
        return line;
    }
    let bytes = content.as_bytes();
    let opening = (0..bytes.len())
        .rev()
        .find(|&i| bytes[i] == b'"' && closes_string(backslashes_before(bytes, i), level));
    match opening {
        Some(quote) => line.min(indent(&content[quote + 1..])),
        None => line,
    }
}

/// Splits `input` into `(content, ending)` pairs at every break, real or escaped, where
/// `ending` is the break or `""` for the last line. Concatenating every pair gives `input`
/// back.
fn lines(input: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = Some(input);
    std::iter::from_fn(move || {
        let current = rest?;
        let line = next_line(current, None);
        rest = (!line.ending.is_empty()).then(|| &current[line.len()..]);
        Some((line.content, line.ending))
    })
}

/// The first line of `rest`.
///
/// Outside a block (`block_level` is `None`) a line ends at every break: a real `\n`, or `n`
/// after a run of backslashes (`\n`, or `\\n` when the text was escaped twice). Inside a block
/// at level `L`, it ends only at a break whose [`escape_level`] is at most `L`, or (for `L >= 1`)
/// right before the quote that closes the string holding the block (see [`closes_string`]).
/// A `\r` just before a break, real or escaped, belongs to it.
fn next_line(rest: &str, block_level: Option<u32>) -> Line<'_> {
    let bytes = rest.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let start = match b {
            b'\n' => i,
            b'n' => match backslashes_before(bytes, i) {
                0 => continue,
                run if block_level.is_none_or(|level| break_level(run) <= level) => i - run,
                _ => continue,
            },
            b'"' => match block_level {
                Some(level) if closes_string(backslashes_before(bytes, i), level) => {
                    let start = i - backslashes_before(bytes, i);
                    return Line {
                        content: &rest[..start],
                        ending: "",
                        closed: true,
                    };
                }
                _ => continue,
            },
            _ => continue,
        };
        let start = carriage_return_before(bytes, start);
        return Line {
            content: &rest[..start],
            ending: &rest[start..=i],
            closed: false,
        };
    }
    Line {
        content: rest,
        ending: "",
        closed: false,
    }
}

/// The escape level of a line break: 0 for a real newline, 1 for `\n` (inside a JSON or
/// `Debug` string), 2 for `\\n` (that string escaped again), and so on.
fn escape_level(ending: &str) -> u32 {
    let bytes = ending.as_bytes();
    match bytes.last() {
        Some(b'n') => break_level(backslashes_before(bytes, bytes.len() - 1)),
        _ => 0,
    }
}

/// The escape level of `n` after `run` backslashes (`run >= 1`). Each escaping doubles the
/// backslashes already there and adds one before the `n`, so the level is read from the
/// lowest set bit: 1 (`\n`) and 3 (`\\` then `\n`) are level 1, 2 (`\\n`) is level 2.
fn break_level(run: usize) -> u32 {
    run.trailing_zeros() + 1
}

/// Whether a `"` after `run` backslashes closes a string that holds a block at `level`.
///
/// A quote escaped for a value inside the block carries at least `level` escapes (`\"` at
/// level 1, `\\\"` at level 2): its run ends in `level` one bits. Fewer means it closes the
/// string at `level` or one of the strings around it. At level 0 there is no such string.
fn closes_string(run: usize, level: u32) -> bool {
    run.trailing_ones() < level
}

/// Number of consecutive backslashes ending right before `end`.
fn backslashes_before(bytes: &[u8], end: usize) -> usize {
    bytes[..end]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
}

/// `start`, moved back over a real `\r` or an escaped one (`r` after backslashes).
fn carriage_return_before(bytes: &[u8], start: usize) -> usize {
    match start.checked_sub(1).map(|i| bytes[i]) {
        Some(b'\r') => start - 1,
        Some(b'r') => match backslashes_before(bytes, start - 1) {
            0 => start,
            run => start - 1 - run,
        },
        _ => start,
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, break_level, closes_string, escape_level, header_indent, lines, next_line};

    #[test]
    fn lines_split_at_real_and_escaped_breaks_and_round_trip() {
        let input = "a\nb\r\nc\\nd\\r\\ne\\\\nf\\\\r\\\\ng";
        let parts: Vec<_> = lines(input).collect();
        assert_eq!(
            parts,
            [
                ("a", "\n"),
                ("b", "\r\n"),
                ("c", "\\n"),
                ("d", "\\r\\n"),
                ("e", "\\\\n"),
                ("f", "\\\\r\\\\n"),
                ("g", "")
            ]
        );
        let joined: String = parts.iter().map(|(c, e)| format!("{c}{e}")).collect();
        assert_eq!(joined, input);
        assert_eq!(lines("").collect::<Vec<_>>(), [("", "")]);
        assert_eq!(lines("x\n").collect::<Vec<_>>(), [("x", "\n"), ("", "")]);
    }

    #[test]
    fn escape_levels_of_breaks_and_quotes() {
        assert_eq!(escape_level("\n"), 0);
        assert_eq!(escape_level("\r\n"), 0);
        assert_eq!(escape_level("\\n"), 1);
        assert_eq!(escape_level("\\r\\n"), 1);
        assert_eq!(escape_level("\\\\n"), 2);
        assert_eq!(escape_level("\\\\r\\\\n"), 2);
        assert_eq!((break_level(1), break_level(2), break_level(3)), (1, 2, 1));
        // Level 1: `"` and `\\"` close the string, `\"` is a quote inside a value.
        assert!(closes_string(0, 1) && closes_string(2, 1) && !closes_string(1, 1));
        // Level 2: `"` and `\"` close a string, `\\\"` is a quote inside a value.
        assert!(closes_string(0, 2) && closes_string(1, 2) && !closes_string(3, 2));
        // Level 0: no string to close.
        assert!(!closes_string(0, 0));
    }

    #[test]
    fn a_block_line_skips_deeper_escapes_and_stops_at_its_closing_quote() {
        let line = |content, ending, closed| Line {
            content,
            ending,
            closed,
        };
        // Real-newline text: `\n` inside a quoted value is not a break.
        assert_eq!(
            next_line("  k: \"a\\nb\"\n  j: c", Some(0)),
            line("  k: \"a\\nb\"", "\n", false)
        );
        // One level down: `\\n` and `\"` belong to the value, the bare `"` ends the string.
        assert_eq!(
            next_line("  k: \\\"a\\\\nb\\\"\",\"t\":1", Some(1)),
            line("  k: \\\"a\\\\nb\\\"", "", true)
        );
        // Two levels down: the `Debug` string's `\"` closes it.
        assert_eq!(
            next_line("  k: \\\\\\\"v\\\\\\\" x\\\" y", Some(2)),
            line("  k: \\\\\\\"v\\\\\\\" x", "", true)
        );
        // Outside a block every escaped break counts and quotes do not.
        assert_eq!(
            next_line("k: \"a\\nb\"", None),
            line("k: \"a", "\\n", false)
        );
    }

    #[test]
    fn a_header_inside_a_string_is_indented_from_its_opening_quote() {
        // Real-newline text: the line's own indent.
        assert_eq!(header_indent("    data:", 0), 4);
        assert_eq!(header_indent(r#"    "manifest": "data:"#, 0), 4);
        // Pretty JSON and `{:#?}` Debug: the string opens after the outer indent.
        assert_eq!(header_indent(r#"    "manifest": "data:"#, 1), 0);
        assert_eq!(header_indent(r#"    manifest: "  data:"#, 1), 2);
        // Two levels: `{:#?}` Debug inside a JSON string, whose string opens at `\"`.
        assert_eq!(header_indent(r#"    manifest: \"data:"#, 2), 0);
        // A quote escaped for a value inside the string is not its opening quote.
        assert_eq!(header_indent(r#"  x: \"y\" data:"#, 1), 2);
        // No opening quote on the line: the header follows a break inside the string.
        assert_eq!(header_indent("  data:", 1), 2);
        // The smaller indent wins, so a doubtful line is redacted rather than leaked.
        assert_eq!(header_indent(r#"  m: "   data:"#, 1), 2);
    }
}
