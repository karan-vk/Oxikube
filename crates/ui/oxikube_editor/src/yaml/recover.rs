//! Line-based re-sync after a syntax error: where the next parser run starts and what it
//! continues. granit-parser stops at its first error, so recovery restarts it on a later line.

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
/// item, anything else a mapping key); the innermost such collection is continued. Lines that
/// match nothing are dropped until one does.
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

/// Where the continuation of the open collection at `depth`, restarted at line `at`, ends: the
/// first later line that belongs to an enclosing open collection (less indented, or for a
/// sequence a key at its own column) or a document marker. Lines that fit no open collection
/// stay inside, so the parser reports them.
pub(crate) fn segment_end(text: &str, at: usize, open: &[OpenBlock], depth: usize) -> usize {
    let Some(target) = open.iter().find(|b| b.depth == depth) else {
        return text.len();
    };
    let col = column(text, target.start);
    let enclosing: Vec<(Shape, usize)> = open
        .iter()
        .filter(|b| b.depth < depth)
        .map(|b| (b.shape, column(text, b.start)))
        .collect();
    let (_, mut ls) = line_at(text, at);
    while ls < text.len() {
        let (line, next) = line_at(text, ls);
        if is_marker(line, "---") || is_marker(line, "...") {
            return ls;
        }
        if let Some((indent, shape)) = content(line) {
            let leaves = indent < col
                || (indent == col && target.shape == Shape::Sequence && shape == Shape::Mapping);
            if leaves && enclosing.contains(&(shape, indent)) {
                return ls;
            }
        }
        ls = next;
    }
    text.len()
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
    fn bom_is_not_a_column() {
        assert_eq!(column("\u{feff}a: b", 3), 0);
        assert_eq!(column("x:\n  a: b", 5), 2);
    }
}
