//! [`matched_ranges`]: match positions as byte ranges, so a view draws the matched characters of
//! a row in the accent colour without working out the character boundaries itself.
//!
//! Positions are the **byte offsets** [`Match::positions`](super::Match::positions) reports,
//! ascending; they survive multi-byte text because each position starts one character, and a
//! highlight covers the whole grapheme a position starts (`e` + a combining accent).

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// The byte ranges of the matched graphemes of `text`, with neighbours merged. A position that
/// is out of range or inside a character is skipped.
pub fn matched_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &start in positions {
        let Some(grapheme) = text
            .get(start..)
            .and_then(|rest| rest.graphemes(true).next())
        else {
            continue;
        };
        let end = start + grapheme.len();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            Some(last) if last.end > start => {}
            _ => ranges.push(start..end),
        }
    }
    ranges
}
