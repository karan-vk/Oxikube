//! The cursor's line, read lexically: is a key or a value being typed, and where do they start?
//!
//! Hover reads the spanned model; completion cannot, because the line being typed is usually not
//! valid YAML yet (`ima`, `restartPolicy: `). So the line is classified here from its text alone,
//! cheaply, before anything is parsed: a comment, a flow collection or a block scalar gets no
//! completions and costs no parse.

use std::ops::Range;

/// What is being typed at the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Site {
    /// A mapping key: `ima`, or nothing yet after the indentation or a `- `.
    Key(KeySite),
    /// The value of `key: `.
    Value(ValueSite),
}

/// A key being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct KeySite {
    /// Start of the cursor's line.
    pub(super) line_start: usize,
    /// Where the key starts: after the indentation and any `- `.
    pub(super) key_start: usize,
    /// The word typed so far: `key_start..cursor`.
    pub(super) cursor: usize,
    /// End of the word the completion replaces: the cursor, or the end of the word it is in.
    pub(super) replace_end: usize,
    /// Whether a `:` follows the word, so the completion must not add one.
    pub(super) colon_follows: bool,
    /// Key column in characters from the line start (the indentation of keys of this mapping).
    pub(super) column: usize,
    /// Whether the key is the first thing after a `- `.
    pub(super) item_start: bool,
    /// End of the line, without its line break.
    pub(super) line_end: usize,
}

impl KeySite {
    /// The word typed before the cursor.
    pub(super) fn prefix<'t>(&self, text: &'t str) -> &'t str {
        &text[self.key_start..self.cursor]
    }
}

/// A value being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ValueSite {
    /// Where the key of the entry starts.
    pub(super) key_start: usize,
    /// What the completion replaces: the value text on the line (without a trailing comment).
    pub(super) replace: Range<usize>,
    /// The cursor.
    pub(super) cursor: usize,
    /// The colon is directly before the cursor: the insertion needs a space first.
    pub(super) space_first: bool,
}

impl ValueSite {
    /// The value typed before the cursor, without an opening quote.
    pub(super) fn prefix<'t>(&self, text: &'t str) -> &'t str {
        let typed = &text[self.replace.start..self.cursor.max(self.replace.start)];
        typed.strip_prefix(['"', '\'']).unwrap_or(typed)
    }
}

/// The line a cursor is on.
struct Line<'t> {
    start: usize,
    /// The line without its break.
    text: &'t str,
}

fn line_at(text: &str, cursor: usize) -> Line<'_> {
    let start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    Line {
        start,
        text: text[start..end].trim_end_matches('\r'),
    }
}

/// Where a `#` starts a comment on `line`: at the line start or after a blank, outside quotes.
fn comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        let after_space = i == 0 || matches!(bytes[i - 1], b' ' | b'\t');
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if (b == b'"' || b == b'\'') && (after_space || bytes[i - 1] == b':') => {
                quote = Some(b);
            }
            None if b == b'#' && after_space => return Some(i),
            None => {}
        }
    }
    None
}

const fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '$')
}

/// Classifies the cursor's position, or `None` where nothing is offered: in a comment, in a
/// flow collection or quoted key, in indentation before text, on a document marker or a tab
/// indent, or in a block scalar.
pub(super) fn site_at(text: &str, cursor: usize) -> Option<Site> {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let line = line_at(text, cursor);
    let col = cursor - line.start;
    let comment = comment_start(line.text);
    if comment.is_some_and(|at| col > at) {
        return None;
    }
    let content_end = comment.unwrap_or(line.text.len());
    let before = &line.text[..col];
    let indent = before.len() - before.trim_start_matches(' ').len();
    if before[..indent].contains('\t') || before.starts_with('\t') {
        return None;
    }
    // Past the indentation and any `- ` markers.
    let mut pos = indent;
    let mut item_start = false;
    while let Some(rest) = before[pos..].strip_prefix('-') {
        if !rest.starts_with(' ') {
            break;
        }
        pos += 1 + (rest.len() - rest.trim_start_matches(' ').len());
        item_start = true;
    }
    let rest = &before[pos..];
    let line_trimmed = line.text[..content_end].trim_end();
    if matches!(line_trimmed, "---" | "...") {
        return None;
    }
    if in_block_scalar(text, line.start, pos.max(indent)) {
        return None;
    }
    if let Some(colon) = value_colon(rest) {
        return value_site(&line, pos, colon, col, content_end);
    }
    if rest.starts_with('-') || !rest.chars().all(is_key_char) {
        return None;
    }
    // What follows the cursor must be the rest of the word, then `:` or the end of the line.
    let after = &line.text[col..content_end];
    let word = after.len() - after.trim_start_matches(is_key_char).len();
    let tail = &after[word..];
    let colon_follows = tail.starts_with(':');
    if !colon_follows && !tail.trim().is_empty() {
        return None;
    }
    Some(Site::Key(KeySite {
        line_start: line.start,
        key_start: line.start + pos,
        cursor,
        replace_end: cursor + word,
        colon_follows,
        column: line.text[..pos].chars().count(),
        item_start,
        line_end: line.start + line.text.len(),
    }))
}

/// The byte index of the `:` ending the key in `rest` (the text before the cursor after the
/// indentation), when one does: followed by a blank or by the cursor, after a plain key.
fn value_colon(rest: &str) -> Option<usize> {
    let colon = rest.find(':')?;
    let next = rest[colon + 1..].chars().next();
    (next.is_none() || next == Some(' ')).then_some(colon)
}

fn value_site(
    line: &Line<'_>,
    pos: usize,
    colon: usize,
    col: usize,
    content_end: usize,
) -> Option<Site> {
    let key = &line.text[pos..pos + colon];
    if key.is_empty() || !key.chars().all(is_key_char) {
        return None;
    }
    let after_colon = pos + colon + 1;
    let spaces =
        line.text[after_colon..].len() - line.text[after_colon..].trim_start_matches(' ').len();
    let start = (after_colon + spaces).min(col);
    let end = line.text[..content_end].trim_end().len().max(col);
    // A block scalar header, a flow collection, an anchor, alias or tag: not a value to complete.
    if line.text[start..col].starts_with(['|', '>', '{', '[', '&', '*', '!']) {
        return None;
    }
    Some(Site::Value(ValueSite {
        key_start: line.start + pos,
        replace: line.start + start..line.start + end,
        cursor: line.start + col,
        space_first: col == after_colon && start == col && spaces == 0,
    }))
}

/// Whether the line starting at `line_start`, with content at column `indent`, is inside a block
/// scalar (`key: |` and friends): the nearest earlier line indented less than it is the header.
fn in_block_scalar(text: &str, line_start: usize, indent: usize) -> bool {
    let mut end = line_start;
    while end > 0 {
        let before = &text[..end - 1];
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        let line = text[start..end - 1].trim_end_matches('\r');
        end = start;
        let trimmed = line.trim_start_matches(' ');
        if trimmed.is_empty() {
            continue;
        }
        let line_indent = line.len() - trimmed.len();
        if line_indent >= indent {
            continue;
        }
        return is_block_header(trimmed);
    }
    false
}

/// `key: |`, `key: >-`, `- |+2`: a line that opens a block scalar.
fn is_block_header(line: &str) -> bool {
    let code = comment_start(line)
        .map_or(line, |at| &line[..at])
        .trim_end();
    let indicator = code.trim_end_matches(|c: char| c.is_ascii_digit() || c == '+' || c == '-');
    let Some(head) = indicator.strip_suffix(['|', '>']) else {
        return false;
    };
    head.ends_with(": ") || head.ends_with(" - ") || head == "- " || head.ends_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `‸` marking the cursor.
    fn site(marked: &str) -> Option<Site> {
        let cursor = marked.find('‸').expect("a cursor");
        let text = marked.replacen('‸', "", 1);
        site_at(&text, cursor)
    }

    fn key(marked: &str) -> KeySite {
        match site(marked) {
            Some(Site::Key(key)) => key,
            other => panic!("not a key site: {other:?}"),
        }
    }

    #[test]
    fn a_half_typed_key() {
        let k = key("spec:\n  containers:\n  - name: a\n    ima‸");
        assert_eq!(k.column, 4);
        assert!(!k.item_start && !k.colon_follows);
        assert_eq!(k.cursor - k.key_start, 3);
    }

    #[test]
    fn a_blank_line_and_a_dash_start_a_key() {
        assert_eq!(key("spec:\n  ‸").column, 2);
        let k = key("containers:\n  - ‸");
        assert!(k.item_start);
        assert_eq!(k.column, 4);
        assert_eq!(key("‸").column, 0);
    }

    #[test]
    fn a_cursor_inside_a_key_replaces_the_rest_of_the_word() {
        let k = key("  ima‸ge: nginx");
        assert!(k.colon_follows);
        assert_eq!(k.replace_end - k.cursor, 2);
    }

    #[test]
    fn a_value_after_the_colon() {
        let marked = "  restartPolicy: Alw‸ays  # why";
        let Some(Site::Value(v)) = site(marked) else {
            panic!("not a value");
        };
        let text = marked.replacen('‸', "", 1);
        assert_eq!(&text[v.replace.clone()], "Always");
        assert_eq!(v.prefix(&text), "Alw");
        assert!(!v.space_first);
        let Some(Site::Value(v)) = site("a:‸") else {
            panic!("not a value");
        };
        assert!(v.space_first);
        let Some(Site::Value(v)) = site("- imagePullPolicy: ‸") else {
            panic!("not a value");
        };
        assert!(!v.space_first);
    }

    #[test]
    fn nothing_is_offered_in_comments_flow_collections_markers_or_quoted_keys() {
        for marked in [
            "a: 1 # com‸",
            "# so‸",
            "a: {b: ‸",
            "a: [x‸",
            "---‸",
            "\"quoted‸",
            "  ‸  foo: bar",
            "-‸",
            "a: |‸",
            "a: >-‸",
            "\tindented‸",
        ] {
            assert_eq!(site(marked), None, "{marked}");
        }
    }

    #[test]
    fn a_hash_in_a_value_is_not_a_comment() {
        assert!(matches!(site("url: http://x/#frag‸"), Some(Site::Value(_))));
        assert!(matches!(site("a: \"x # y‸"), Some(Site::Value(_))));
    }

    #[test]
    fn block_scalars_are_skipped_and_their_siblings_are_not() {
        assert_eq!(site("data:\n  script: |\n    ec‸ho"), None);
        assert_eq!(site("data:\n  script: |\n    echo\n    ima‸"), None);
        assert_eq!(site("data:\n  script: >-\n\n    ec‸ho"), None);
        assert!(site("data:\n  script: |\n    echo\n  ima‸").is_some());
        assert!(site("args:\n  - |\n    ec‸ho").is_none());
    }
}
