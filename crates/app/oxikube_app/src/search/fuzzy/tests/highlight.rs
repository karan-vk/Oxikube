//! The highlight helper: positions as byte ranges.

use crate::search::fuzzy::highlight::matched_ranges;

#[test]
fn neighbours_merge_and_bad_offsets_are_skipped() {
    assert_eq!(matched_ranges("abcdef", &[0, 1, 3, 99]), [0..2, 3..4]);
    // 1 is inside "é".
    assert!(matched_ranges("é", &[1]).is_empty());
    assert_eq!(matched_ranges("aé", &[0, 1]), [0..3]);
    // A repeated offset does not extend a run.
    assert_eq!(matched_ranges("abc", &[1, 1]), [1..2]);
}

#[test]
fn ranges_start_and_end_on_character_boundaries() {
    let text = "ünïcode-pod";
    for range in matched_ranges(text, &[0, 10, 11, 12]) {
        assert!(text.is_char_boundary(range.start));
        assert!(text.is_char_boundary(range.end));
    }
}
