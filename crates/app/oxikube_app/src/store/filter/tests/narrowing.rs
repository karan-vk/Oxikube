//! When a filter narrows another (so the result set can be derived from the previous one), and
//! the property that deriving it always agrees with a fresh full filter.

use proptest::prelude::*;

use super::parts;
use crate::store::StoreFilter;

fn filter(input: &str) -> StoreFilter {
    parts(input).filter
}

#[test]
fn appending_to_a_substring_or_fuzzy_query_narrows() {
    assert!(filter("web").narrows(&filter("we")));
    assert!(filter("web").narrows(&filter("")));
    assert!(filter("web").narrows(&filter("web")));
    assert!(!filter("we").narrows(&filter("web")), "deleting widens");
    assert!(!filter("db").narrows(&filter("web")));
    assert!(filter("-f wbp").narrows(&filter("-f wb")));
    assert!(!filter("-f wb").narrows(&filter("-f wbp")));
}

#[test]
fn inverse_narrows_when_its_text_shrinks() {
    assert!(filter("!we").narrows(&filter("!web")));
    assert!(!filter("!web").narrows(&filter("!we")));
    assert!(
        !filter("!web").narrows(&filter("web")),
        "different polarity"
    );
}

#[test]
fn regexes_and_unrelated_changes_are_recomputed() {
    assert!(!filter("a|b").narrows(&filter("a")));
    assert!(
        filter("a.").narrows(&filter("a.")),
        "the same regex is trivially narrower"
    );
    assert!(
        !filter("web").narrows(&filter("^we")),
        "mixing regex and text"
    );
    assert!(
        !filter("").narrows(&filter("web")),
        "dropping the filter widens"
    );
    let mut other = filter("web");
    other.name = Some("x".into());
    assert!(!other.narrows(&filter("we")), "another field changed too");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Typing and deleting characters, filtering the previous result set whenever the new filter
    /// narrows it and recomputing otherwise, ends with the same set as a fresh full filter at
    /// every step.
    #[test]
    fn incremental_filtering_equals_a_fresh_filter(
        names in proptest::collection::vec("[a-c-]{1,8}", 1..40),
        edits in proptest::collection::vec((any::<bool>(), "[a-c]"), 1..14),
        mode in 0u8..4,
    ) {
        let prefix = ["", "!", "-f ", "!-f "][usize::from(mode)];
        let mut text = String::new();
        let mut current: Vec<&String> = names.iter().collect();
        let mut last = filter(prefix);
        for (add, ch) in edits {
            if add { text.push_str(&ch) } else { text.pop(); }
            let next = filter(&format!("{prefix}{text}"));
            let fresh: Vec<&String> = names
                .iter()
                .filter(|n| next.pattern.as_ref().is_none_or(|p| p.matches(n)))
                .collect();
            let derived: Vec<&String> = if next.narrows(&last) {
                current
                    .iter()
                    .copied()
                    .filter(|n| next.pattern.as_ref().is_none_or(|p| p.matches(n)))
                    .collect()
            } else {
                fresh.clone()
            };
            prop_assert_eq!(&derived, &fresh, "after editing to {:?}", text);
            current = derived;
            last = next;
        }
    }
}
