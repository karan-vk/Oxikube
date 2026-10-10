//! Line-based re-sync after a syntax error: where the next parser run starts and what it
//! continues. granit-parser stops at its first error, so recovery restarts it on a later line.

use std::collections::HashMap;

use super::builder::{OpenBlock, Shape};

/// Where to restart after an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resume {
    /// Restart at line `at`; its content continues the open block collection at `depth`.
    Attach { at: usize, depth: usize },
    /// Restart at line `at`; its content becomes the root of the current, still empty, document.
    Adopt { at: usize },
    /// Restart at `at` with new documents (a `---` line, or the line after `...`).
    NewDoc { at: usize },
}

/// The first offset to look for a restart line: the start of the line after `last_end` (the end
/// of the last event the failed run produced), or `last_end` itself when it is a line start.
/// Restarting no earlier than that never re-parses text already in the tree.
pub(crate) fn scan_start(text: &str, last_end: usize) -> usize {
    if last_end == 0 || text.as_bytes().get(last_end - 1) == Some(&b'\n') {
        return last_end;
    }
    text.get(last_end..)
        .and_then(|rest| rest.find('\n'))
        .map_or(text.len(), |i| last_end + i + 1)
}

/// Finds the restart line at or after `from` and strictly after `run_start` (so every run makes
/// progress). Blank, comment-only and tab-indented lines are skipped. A line restarts the parse
/// when it is a document marker, when the document has no node yet, or when its indentation
/// equals the column of an open block collection of the same shape (`- ` starts a sequence
/// item, a line with a key indicator a mapping entry); the innermost such collection is
/// continued. Lines that match nothing are dropped until one does, among them a half-typed key
/// with no `:` yet: restarting on it would fold the next line into a plain scalar with it.
pub(crate) fn find_resume(
    text: &str,
    from: usize,
    run_start: usize,
    open: &[OpenBlock],
    doc_empty: bool,
) -> Option<Resume> {
    let mut ls = from;
    while ls < text.len() {
        let (line, next) = line_at(text, ls);
        if ls <= run_start {
            ls = next;
            continue;
        }
        if is_marker(line, "---") {
            return Some(Resume::NewDoc { at: ls });
        }
        if is_marker(line, "...") {
            return (next < text.len()).then_some(Resume::NewDoc { at: next });
        }
        if let Some((indent, shape)) = content(line) {
            if doc_empty {
                return Some(Resume::Adopt { at: ls });
            }
            if shape == Shape::Mapping && !starts_entry(&line[indent..]) {
                ls = next;
                continue;
            }
            let target = open
                .iter()
                .rev()
                .find(|b| b.shape == shape && column(text, b.start) == indent);
            if let Some(block) = target {
                return Some(Resume::Attach {
                    at: ls,
                    depth: block.depth,
                });
            }
        }
        ls = next;
    }
    None
}

/// Memoised [`segment_end`] results for one parse, so recovery stays linear in the buffer size.
///
/// The end of a collection's continuation depends only on the collection (its ancestors are the
/// enclosing ones) and holds for every restart line before it. Restarts move forward, so a
/// collection hit by many errors (the root mapping, or the `items:` list of a large dump) is
/// scanned once instead of once per error.
#[derive(Debug, Default)]
pub(crate) struct SegmentEnds {
    /// `(collection start, depth)` -> end of its continuation.
    known: HashMap<(usize, usize), usize>,
    /// Lines [`segment_end`] has looked at, for tests.
    #[cfg(test)]
    scanned: usize,
}

impl SegmentEnds {
    /// [`segment_end`], reusing an earlier scan of the same collection when `at` is before it.
    pub fn get(&mut self, text: &str, at: usize, open: &[OpenBlock], depth: usize) -> usize {
        let Some(target) = open.iter().find(|b| b.depth == depth) else {
            return text.len();
        };
        let key = (target.start, depth);
        if let Some(&end) = self.known.get(&key)
            && at < end
        {
            return end;
        }
        let (end, _scanned) = segment_end(text, at, open, target);
        #[cfg(test)]
        {
            self.scanned += _scanned;
        }
        self.known.insert(key, end);
        end
    }
}

/// Where the continuation of the open collection `target`, restarted at line `at`, ends: the
/// first later line that belongs to an enclosing open collection (less indented, or for a
/// sequence a key at its own column) or a document marker. Lines that fit no open collection
/// stay inside, so the parser reports them. Also returns how many lines it scanned.
fn segment_end(text: &str, at: usize, open: &[OpenBlock], target: &OpenBlock) -> (usize, usize) {
    let depth = target.depth;
    let col = column(text, target.start);
    let enclosing: Vec<(Shape, usize)> = open
        .iter()
        .filter(|b| b.depth < depth)
        .map(|b| (b.shape, column(text, b.start)))
        .collect();
    let (_, mut ls) = line_at(text, at);
    let mut scanned = 0;
    while ls < text.len() {
        scanned += 1;
        let (line, next) = line_at(text, ls);
        if is_marker(line, "---") || is_marker(line, "...") {
            return (ls, scanned);
        }
        if let Some((indent, shape)) = content(line) {
            let leaves = indent < col
                || (indent == col && target.shape == Shape::Sequence && shape == Shape::Mapping);
            if leaves && enclosing.contains(&(shape, indent)) {
                return (ls, scanned);
            }
        }
        ls = next;
    }
    (text.len(), scanned)
}

/// The line starting at `ls` (without its line break) and the start of the next one.
fn line_at(text: &str, ls: usize) -> (&str, usize) {
    let rest = text.get(ls..).unwrap_or("");
    let len = rest.find('\n').unwrap_or(rest.len());
    (rest[..len].trim_end_matches('\r'), ls + len + 1)
}

/// A content line's indentation and whether it starts a sequence item (`- `) or a mapping key;
/// `None` for blank, comment-only and tab-indented lines.
fn content(line: &str) -> Option<(usize, Shape)> {
    let rest = line.trim_start_matches(' ');
    if rest.is_empty() || rest.starts_with('#') || rest.starts_with('\t') {
        return None;
    }
    let item = rest == "-" || rest.starts_with("- ") || rest.starts_with("-\t");
    let shape = if item {
        Shape::Sequence
    } else {
        Shape::Mapping
    };
    Some((line.len() - rest.len(), shape))
}

/// Whether a line (without its indentation) can start a block mapping entry: an explicit `?` key
/// or `:` value, or an implicit key followed by a `:` indicator (`:` then a space, a tab or the
/// line end) before any comment. A quoted key is skipped whole, so a `: ` inside it does not count.
fn starts_entry(rest: &str) -> bool {
    if is_marker(rest, "?") || is_marker(rest, ":") {
        return true;
    }
    let bytes = rest.as_bytes();
    let mut i = match bytes.first() {
        Some(&quote @ (b'"' | b'\'')) => quoted_end(bytes, quote),
        _ => 0,
    };
    while i < bytes.len() {
        match bytes[i] {
            b':' if matches!(bytes.get(i + 1), None | Some(b' ' | b'\t')) => return true,
            b'#' if i > 0 && matches!(bytes[i - 1], b' ' | b'\t') => return false,
            _ => {}
        }
        i += 1;
    }
    false
}

/// The offset after the closing quote of a scalar opening `bytes`, or its length when the
/// quote is not closed on this line.
fn quoted_end(bytes: &[u8], quote: u8) -> usize {
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quote == b'"' => i += 1,
            b'\'' if quote == b'\'' && bytes.get(i + 1) == Some(&b'\'') => i += 1,
            b if b == quote => return i + 1,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

fn is_marker(line: &str, marker: &str) -> bool {
    line.strip_prefix(marker)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
}

/// Byte column of `offset` in its line (a leading UTF-8 BOM is not a column).
fn column(text: &str, offset: usize) -> usize {
    let start = text
        .get(..offset)
        .and_then(|before| before.rfind('\n'))
        .map_or_else(
            || if text.starts_with('\u{feff}') { 3 } else { 0 },
            |i| i + 1,
        );
    offset.saturating_sub(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_starts_on_the_next_line() {
        let text = "a: b\nc: d\n";
        assert_eq!(scan_start(text, 0), 0);
        assert_eq!(scan_start(text, 4), 5);
        assert_eq!(scan_start(text, 5), 5);
        assert_eq!(scan_start(text, 9), 10);
    }

    #[test]
    fn markers_need_a_separator() {
        assert!(is_marker("---", "---"));
        assert!(is_marker("--- # c", "---"));
        assert!(!is_marker("----", "---"));
        assert!(!is_marker(" ---", "---"));
    }

    #[test]
    fn entry_lines_need_a_key_indicator() {
        for entry in [
            "a: b",
            "a:",
            "a:\tb",
            "? a",
            "?",
            ": b",
            "\"a: b\": c",
            "'it''s': c",
            "url: http://x",
            "a: b # c: d",
        ] {
            assert!(starts_entry(entry), "{entry:?}");
        }
        for not_entry in [
            "lab",
            "http://x",
            "a:b",
            "lab # note: x",
            "\"a: b\"",
            "'a: b'",
            "\"unclosed: x",
        ] {
            assert!(!starts_entry(not_entry), "{not_entry:?}");
        }
    }

    #[test]
    fn half_typed_key_is_not_a_restart_line() {
        // Open: the root mapping (column 0) and `metadata` (column 2).
        let text = "metadata:\n  name: web\n  lab\n  namespace: x\nspec: 1\n";
        let open = [block(0, Shape::Mapping, 0), block(1, Shape::Mapping, 12)];
        let lab = text.find("  lab").unwrap();
        assert_eq!(
            find_resume(text, lab, 0, &open, false),
            Some(Resume::Attach {
                at: text.find("  namespace").unwrap(),
                depth: 1
            })
        );
    }

    #[test]
    fn segment_scans_are_reused_across_errors() {
        let text = "k: v\n".repeat(1_000);
        let open = [block(0, Shape::Mapping, 0)];
        let mut ends = SegmentEnds::default();
        assert_eq!(ends.get(&text, 5, &open, 0), text.len());
        let first = ends.scanned;
        assert!(first >= 998, "{first}");
        // Later restarts in the same collection (one per error) do not rescan the buffer.
        for line in 2..1_000 {
            assert_eq!(ends.get(&text, line * 5, &open, 0), text.len());
        }
        assert_eq!(ends.scanned, first);
    }

    #[test]
    fn segment_ends_at_an_enclosing_line() {
        let text = "a:\n  b: 1\n  c: 2\nd: 3\n";
        let open = [block(0, Shape::Mapping, 0), block(1, Shape::Mapping, 5)];
        let mut ends = SegmentEnds::default();
        let d = text.find("d:").unwrap();
        assert_eq!(ends.get(text, text.find("  c").unwrap(), &open, 1), d);
        // A restart past the cached end scans again.
        assert_eq!(ends.get(text, d, &open, 1), text.len());
    }

    fn block(depth: usize, shape: Shape, start: usize) -> OpenBlock {
        OpenBlock {
            depth,
            shape,
            start,
        }
    }

    #[test]
    fn bom_is_not_a_column() {
        assert_eq!(column("\u{feff}a: b", 3), 0);
        assert_eq!(column("x:\n  a: b", 5), 2);
    }
}
