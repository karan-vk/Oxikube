//! The `/` filter over rows.

use crate::rbac_subjects::SubjectFilter;

#[test]
fn a_plain_filter_matches_any_field_and_an_inverted_one_none() {
    let rows = [["fx-viewer", "User"], ["fx-team", "Group"], ["bob", "User"]];
    let shown = |text: &str| -> Vec<&str> {
        let filter = SubjectFilter::parse(text);
        rows.iter()
            .filter(|row| filter.matches(&row[..]))
            .map(|row| row[0])
            .collect()
    };
    assert_eq!(shown(""), ["fx-viewer", "fx-team", "bob"]);
    assert_eq!(shown("fx-"), ["fx-viewer", "fx-team"]);
    assert_eq!(shown("group"), ["fx-team"]);
    assert_eq!(shown("!fx"), ["bob"]);
    assert_eq!(shown("^b.b$"), ["bob"]);
}

#[test]
fn a_filter_that_does_not_parse_lets_everything_through_and_says_why() {
    let broken = SubjectFilter::parse("(");
    assert!(broken.is_empty());
    assert!(broken.problem().is_some());
    assert!(broken.matches(&["anything"]));
    let selector = SubjectFilter::parse("-l app=x");
    assert!(selector.problem().is_some());
}
