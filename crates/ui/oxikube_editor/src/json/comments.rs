//! Counting the comments of a YAML buffer, for the "N comments dropped" warning.
//!
//! A `#` starts a comment when it opens the text or follows white space, and is not inside a
//! scalar. The spanned model gives the scalars' spans (quotes included, block scalar content
//! included), so a `#` in `"a #b"`, in a block scalar's script or in `image: a#b` is not counted,
//! while one in the `|` header line or after a flow entry is. A comment runs to the end of its
//! line and counts once, however many `#` it holds.

use std::ops::Range;

use crate::yaml::{NodeKind, ParseResult};

/// How many comments `parsed`'s text holds.
#[must_use]
pub fn count_comments(parsed: &ParseResult) -> usize {
    let text = parsed.text();
    if !text.contains('#') {
        return 0;
    }
    // Scalar and alias spans in source order (documents and nodes are both in source order).
    let spans: Vec<Range<usize>> = parsed
        .docs()
        .iter()
        .flat_map(|doc| doc.nodes())
        .filter(|node| matches!(node.kind, NodeKind::Scalar(_) | NodeKind::Alias { .. }))
        .map(|node| node.span.clone())
        .filter(|span| !span.is_empty())
        .collect();
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find('#') {
        let at = from + found;
        from = at + 1;
        let opens_comment = at == 0 || bytes[at - 1].is_ascii_whitespace();
        let inside = {
            let after = spans.partition_point(|span| span.start <= at);
            after > 0 && at < spans[after - 1].end
        };
        if opens_comment && !inside {
            count += 1;
            from = text[at..].find('\n').map_or(text.len(), |n| at + n);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::parse;

    fn count(text: &str) -> usize {
        count_comments(&parse(text))
    }

    #[test]
    fn counts_full_line_and_trailing_comments() {
        assert_eq!(count("# top\na: 1 # one\n# mid\nb: 2\n"), 3);
        assert_eq!(count("a: 1\n"), 0);
        assert_eq!(count(""), 0);
    }

    #[test]
    fn a_comment_with_more_hashes_counts_once() {
        assert_eq!(count("a: 1 # see #42 ## and #more\n"), 1);
    }

    #[test]
    fn hashes_inside_scalars_are_not_comments() {
        let yaml = "\
a: \"x #y\"\n\
b: 'p # q'\n\
c: a#b\n\
d: |\n  echo hi # not a comment\n  # nor this\n\
e: >-\n  folded # text\n\
f: plain\n";
        assert_eq!(count(yaml), 0);
    }

    #[test]
    fn the_block_scalar_header_line_can_hold_a_comment() {
        assert_eq!(count("a: | # explained\n  text # kept\nb: 1\n"), 1);
    }

    #[test]
    fn comments_in_flow_collections_and_between_documents_count() {
        assert_eq!(count("a: [1, # one\n  2]\n---\n# second doc\nb: 1\n"), 2);
        assert_eq!(count("a: {x: 1, # y\n  z: 2}\n"), 1);
    }

    #[test]
    fn a_hash_after_an_alias_or_anchor_name_is_not_a_comment() {
        assert_eq!(count("a: &x 1\nb: *x # yes this is\n"), 1);
    }
}
