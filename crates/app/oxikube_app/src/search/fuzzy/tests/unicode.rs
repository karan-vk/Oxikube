//! Non-ASCII titles: normalisation on, positions are byte offsets of whole characters.

use super::{rank, texts};
use crate::search::fuzzy::highlight::matched_ranges;

#[test]
fn accents_are_normalised_one_way() {
    let candidates = ["Café Latte", "Cafe Mocha", "Tea"];
    // An ASCII query finds the accented title.
    let found = rank("cafe", &candidates);
    let order = texts(&found, &candidates);
    assert!(order.contains(&"Café Latte"), "{order:?}");
    assert!(order.contains(&"Cafe Mocha"), "{order:?}");
    // An accented query does not match the plain one ("ä != a" in nucleo's rule).
    let accented = rank("café", &candidates);
    assert_eq!(texts(&accented, &candidates), ["Café Latte"]);
}

#[test]
fn positions_are_byte_offsets_for_multi_byte_text() {
    let candidates = ["ünïcode-pod"];
    let found = rank("pod", &candidates);
    assert_eq!(found[0].positions, [10, 11, 12]);
    for &at in &found[0].positions {
        assert!(candidates[0].is_char_boundary(at));
    }
}

#[test]
fn matched_characters_are_the_ones_highlighted_in_cjk_and_emoji() {
    let candidates = ["ポッド 削除", "🚀 pod launch", "Node 排水"];
    for (query, expected) in [("削除", "削除"), ("pod", "pod"), ("排水", "排水")] {
        let found = rank(query, &candidates);
        assert_eq!(found.len(), 1, "{query}");
        let text = candidates[found[0].index];
        let matched: String = found[0]
            .positions
            .iter()
            .map(|&at| text[at..].chars().next().unwrap())
            .collect();
        assert_eq!(matched, expected);
    }
}

#[test]
fn matched_characters_spell_the_query_in_ascii_text() {
    let candidates = ["Workload Rollout Restart", "Port Forward Stop"];
    let found = rank("wrr", &candidates);
    let m = &found[0];
    let spelled: String = m
        .positions
        .iter()
        .map(|&at| {
            candidates[m.index][at..]
                .chars()
                .next()
                .unwrap()
                .to_ascii_lowercase()
        })
        .collect();
    assert_eq!(spelled, "wrr");
}

#[test]
fn positions_count_graphemes_when_an_accent_is_a_combining_mark() {
    // Every grapheme starts with an ASCII char, so nucleo matches the raw bytes.
    let decomposed = "cafe\u{301}-pod";
    let found = rank("pod", &[decomposed]);
    assert_eq!(found[0].positions, [7, 8, 9]);
    assert_eq!(matched_ranges(decomposed, &found[0].positions), [7..10]);

    // A non-ASCII grapheme start: nucleo matches one char per grapheme.
    let mixed = "\u{fc}e\u{301}-pod";
    let found = rank("pod", &[mixed]);
    assert_eq!(found[0].positions, [6, 7, 8]);
    assert_eq!(matched_ranges(mixed, &found[0].positions), [6..9]);
}

#[test]
fn a_highlight_covers_the_whole_grapheme() {
    let found = rank("cafe", &["cafe\u{301}"]);
    assert_eq!(found[0].positions, [0, 1, 2, 3]);
    assert_eq!(matched_ranges("cafe\u{301}", &found[0].positions), [0..6]);
}
