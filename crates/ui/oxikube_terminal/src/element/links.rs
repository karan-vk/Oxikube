//! Finding the link under the pointer: the cell's OSC 8 hyperlink, else a URL or a file path
//! (`src/main.rs:12:3`) the regexes find in the hovered line.
//!
//! Detection runs only when the pointer moves to another cell with the platform modifier held
//! (cmd on macOS, ctrl elsewhere), and only over the hovered logical line: the viewport rows
//! joined by soft wraps around the hovered one. Nothing runs per frame.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::grid::{CellFlags, TerminalSnapshot};

/// URLs: a scheme the opener accepts, then anything up to whitespace or a delimiter.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:(?:https?|file|ftp)://|mailto:)[^\s<>"'`{}|\\^]+"#).expect("URL regex")
});

/// Path-like words, with an optional `:line` / `:line:column` suffix.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:~|\.{1,2})?/?[\w.@+\-]+(?:/[\w.@+\-]+)*/?(?::\d+(?::\d+)?)?")
        .expect("path regex")
});

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// A URL (from OSC 8 or found in the text).
    Url,
    /// A file path found in the text, as printed (maybe relative, maybe with `:line:col`).
    Path,
}

/// A link on screen: what it opens and the cells it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLink {
    /// The URL or path text.
    pub target: String,
    /// URL or path.
    pub kind: LinkKind,
    /// The cells it covers, as `(viewport row, first column, last column)` per row.
    pub cells: Vec<(usize, usize, usize)>,
}

impl TerminalLink {
    /// Whether viewport cell `row`, `column` is part of the link.
    pub fn covers(&self, row: usize, column: usize) -> bool {
        self.cells
            .iter()
            .any(|&(r, first, last)| r == row && (first..=last).contains(&column))
    }

    /// The command target for this link: the URL, or the path made absolute against `base`
    /// (`~` against the home directory). `None` for a relative path without a base.
    pub fn resolve(&self, base: Option<&Path>) -> Option<String> {
        if self.kind == LinkKind::Url {
            return Some(self.target.clone());
        }
        let path = &self.target;
        let resolved: PathBuf = if let Some(rest) = path.strip_prefix("~/") {
            std::env::var_os("HOME").map(PathBuf::from)?.join(rest)
        } else if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            base?.join(path)
        };
        Some(resolved.to_string_lossy().into_owned())
    }
}

/// The link at viewport `row`, `column`: the cell's OSC 8 hyperlink, else a URL, else (when
/// `paths`) a path in the hovered logical line.
pub fn link_at(
    snapshot: &TerminalSnapshot,
    row: usize,
    column: usize,
    paths: bool,
) -> Option<TerminalLink> {
    if let Some(link) = hyperlink_at(snapshot, row, column) {
        return Some(link);
    }
    let line = LogicalLine::around(snapshot, row);
    let byte = line.byte_at(row, column)?;
    if let Some(found) = URL
        .find_iter(&line.text)
        .map(|m| trim_url(&line.text, m.start(), m.end()))
        .find(|range| range.contains(&byte))
    {
        return Some(line.link(found, LinkKind::Url));
    }
    if !paths {
        return None;
    }
    PATH.find_iter(&line.text)
        .find(|m| (m.start()..m.end()).contains(&byte))
        .filter(|m| is_path(m.as_str()))
        .map(|m| line.link(m.start()..m.end(), LinkKind::Path))
}

/// The OSC 8 link of a cell, with every cell around it (same row, wrapped rows) that carries the
/// same URI.
fn hyperlink_at(snapshot: &TerminalSnapshot, row: usize, column: usize) -> Option<TerminalLink> {
    let uri = snapshot.hyperlink_at(row, column)?;
    let same = |index: usize| {
        let (row, column) = (index / snapshot.columns, index % snapshot.columns);
        snapshot.hyperlink_at(row, column) == Some(uri)
    };
    let here = row * snapshot.columns + column;
    let mut first = here;
    while first > 0 && same(first - 1) {
        first -= 1;
    }
    let mut last = here;
    while last + 1 < snapshot.cells.len() && same(last + 1) {
        last += 1;
    }
    let cells = (first / snapshot.columns..=last / snapshot.columns)
        .map(|r| {
            let from = if r == first / snapshot.columns {
                first % snapshot.columns
            } else {
                0
            };
            let to = if r == last / snapshot.columns {
                last % snapshot.columns
            } else {
                snapshot.columns - 1
            };
            (r, from, to)
        })
        .collect();
    Some(TerminalLink {
        target: uri.to_string(),
        kind: LinkKind::Url,
        cells,
    })
}

/// `start..end` without trailing punctuation that usually ends a sentence, and without a closing
/// bracket the URL did not open.
fn trim_url(text: &str, start: usize, mut end: usize) -> std::ops::Range<usize> {
    loop {
        let url = &text[start..end];
        let Some(last) = url.chars().last() else {
            break;
        };
        let unbalanced = |open: char, close: char| {
            last == close && url.matches(open).count() < url.matches(close).count()
        };
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"')
            || unbalanced('(', ')')
            || unbalanced('[', ']')
        {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    start..end
}

/// Whether a path-regex match is worth a link: it has a directory part, or it is a file name
/// with an extension and a `:line` suffix (compiler output). Bare words and numbers are not.
fn is_path(text: &str) -> bool {
    let (path, has_line) = match text.split_once(':') {
        Some((path, _)) => (path, true),
        None => (text, false),
    };
    let named = path.chars().any(char::is_alphabetic);
    let has_dir = path.trim_end_matches('/').contains('/') || path.starts_with('~');
    let file_with_line = has_line && path.contains('.') && !path.ends_with('.');
    named && (has_dir || file_with_line)
}

/// The viewport rows joined by soft wraps around one row, as text with a map back to cells.
struct LogicalLine {
    text: String,
    /// For every character of `text`: its byte offset, viewport row and column.
    chars: Vec<(usize, usize, usize)>,
}

impl LogicalLine {
    fn around(snapshot: &TerminalSnapshot, row: usize) -> Self {
        let wraps = |row: usize| {
            snapshot
                .cell(row, snapshot.columns.saturating_sub(1))
                .is_some_and(|cell| cell.flags.contains(CellFlags::WRAPLINE))
        };
        let mut first = row;
        while first > 0 && wraps(first - 1) {
            first -= 1;
        }
        let mut last = row;
        while last + 1 < snapshot.rows && wraps(last) {
            last += 1;
        }
        let spacers = CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER;
        let mut line = Self {
            text: String::new(),
            chars: Vec::new(),
        };
        for r in first..=last {
            for column in 0..snapshot.columns {
                let Some(cell) = snapshot.cell(r, column) else {
                    continue;
                };
                if cell.flags.intersects(spacers) {
                    continue;
                }
                line.chars.push((line.text.len(), r, column));
                line.text.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
        }
        line
    }

    /// The byte offset of viewport cell `row`, `column` (a wide glyph's spacer maps to it).
    fn byte_at(&self, row: usize, column: usize) -> Option<usize> {
        self.chars
            .iter()
            .rev()
            .find(|&&(_, r, c)| r == row && c <= column)
            .map(|&(byte, _, _)| byte)
    }

    fn link(&self, bytes: std::ops::Range<usize>, kind: LinkKind) -> TerminalLink {
        let mut cells: Vec<(usize, usize, usize)> = Vec::new();
        for &(_, row, column) in self
            .chars
            .iter()
            .filter(|(byte, _, _)| bytes.contains(byte))
        {
            match cells.last_mut() {
                Some(last) if last.0 == row => last.2 = column,
                _ => cells.push((row, column, column)),
            }
        }
        TerminalLink {
            target: self.text[bytes].to_owned(),
            kind,
            cells,
        }
    }
}
