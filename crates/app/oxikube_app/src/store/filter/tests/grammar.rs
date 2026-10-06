//! Each syntax form, escaping, empty input, invalid regex, unicode.

use super::*;
use crate::store::filter::{FilterError, TextPattern};
use crate::store::{LabelSelector, LabelTerm};

#[test]
fn empty_and_pending_inputs_are_no_filter() {
    for input in [
        "", "   ", "/", " / ", "!", "/!", "-", "/-", "! ", "!-f", "/!-f", "!-f  ", "! -f",
    ] {
        assert_eq!(ok(input), FilterExpr::Empty, "{input:?}");
        assert!(parts(input).is_empty(), "{input:?}");
    }
}

#[test]
fn plain_text_is_a_case_insensitive_substring() {
    let expr = ok("Web");
    assert!(matches!(expr, FilterExpr::Text(TextPattern::Substring(_))));
    assert!(passes("Web", "my-WEB-app"));
    assert!(passes("web", "WEB-1"));
    assert!(!passes("web", "db-1"));
    assert!(!parts("web").ranks());
}

#[test]
fn a_leading_slash_is_ignored() {
    assert_eq!(ok("/web"), ok("web"));
    assert_eq!(ok("  /web  "), ok("web"));
    assert_eq!(ok("/!web"), ok("!web"));
    assert_eq!(ok("/-f wb"), ok("-f wb"));
    assert_eq!(ok("/-l app=x"), ok("-l app=x"));
}

#[test]
fn regex_syntax_makes_a_regex() {
    assert!(matches!(
        ok("^web-[0-9]+$"),
        FilterExpr::Text(TextPattern::Regex { .. })
    ));
    assert!(passes("^web-[0-9]+$", "web-12"));
    assert!(!passes("^web-[0-9]+$", "web-x"));
    assert!(!passes("^web-[0-9]+$", "my-web-1"));
    assert!(passes("web|db", "db-0"));
    assert!(passes("WEB.*", "web-1"), "regexes are case-insensitive too");
}

#[test]
fn inverse_negates_text_and_fuzzy() {
    let parts = parts("!web");
    let pattern = parts.filter.pattern.as_ref().expect("a pattern");
    assert!(pattern.is_inverse());
    assert!(!pattern.matches("web-1"));
    assert!(pattern.matches("db-1"));
    assert!(passes("!^web", "db-1"));
    assert!(!passes("! ^web", "web-1"), "space after ! is allowed");
    assert!(!passes("!-f wb", "web"));
    assert!(passes("!-f wb", "db"));
    assert!(!super::parts("!-f wb").ranks(), "an inverse has no ranking");
}

#[test]
fn a_label_selector_goes_to_the_server() {
    let parts = parts("-l app=web,tier!=db");
    assert_eq!(
        parts.selector.as_ref().map(ToString::to_string).as_deref(),
        Some("app=web,tier!=db")
    );
    assert!(parts.filter.is_empty(), "nothing is filtered client-side");
    assert_eq!(
        ok("-l  app in (a, b) ").parts().selector,
        Some(LabelSelector::from_terms(vec![LabelTerm::In(
            "app".into(),
            vec!["a".into(), "b".into()]
        )]))
    );
    assert_eq!(parts_of_empty_selector(), None, "an empty selector is none");
}

fn parts_of_empty_selector() -> Option<LabelSelector> {
    parts("-l").selector
}

#[test]
fn fuzzy_is_a_ranked_subsequence() {
    let parts = parts("-f wbp");
    assert!(parts.ranks());
    let pattern = parts.filter.pattern.as_ref().expect("a pattern");
    assert!(pattern.matches("web-pod"));
    assert!(!pattern.matches("pod-web"));
    assert_eq!(
        parts.sort(None).field,
        crate::store::SortField::Relevance,
        "no column sort: best match first"
    );
    assert_eq!(
        parts.sort(Some(crate::store::SortKey::by(
            crate::store::SortField::Name
        ))),
        crate::store::SortKey::by(crate::store::SortField::Name),
        "a column sort wins"
    );
    assert!(
        super::parts("-f").is_empty(),
        "an empty query matches everything"
    );
}

#[test]
fn escapes_make_the_pattern_literal() {
    assert!(passes(r"web\.1", "web.1"));
    assert!(!passes(r"web\.1", "webx1"));
    assert!(passes(r"\!x", "a!x"), "a name pattern may start with !");
    assert!(passes(r"\-x", "a-x"), "or with -");
    assert!(passes(r"a\|b", "a|b"));
    assert!(!passes(r"a\|b", "a"));
}

#[test]
fn invalid_input_is_an_error_not_a_filter() {
    assert!(matches!(parse("web("), Err(FilterError::Regex(m)) if m.contains("unclosed group")));
    assert!(matches!(parse("[a-"), Err(FilterError::Regex(_))));
    assert!(matches!(parse("*web"), Err(FilterError::Regex(_))));
    assert!(matches!(
        parse("-x foo"),
        Err(FilterError::UnknownFlag(f)) if f == "-x"
    ));
    assert!(matches!(parse("-lapp=x"), Err(FilterError::UnknownFlag(_))));
    assert!(matches!(
        parse("-l zone in ("),
        Err(FilterError::Selector(_))
    ));
    assert!(matches!(
        parse("!-l app=x"),
        Err(FilterError::InvertedSelector)
    ));
    let message = parse("web(").unwrap_err().to_string();
    assert!(message.starts_with("invalid regex"), "{message}");
    assert!(!message.contains('\n'), "one line for the bar: {message}");
}

#[test]
fn regexes_have_a_size_limit() {
    assert!(matches!(
        parse("(((a{1000}){1000}){1000})"),
        Err(FilterError::Regex(_))
    ));
}

#[test]
fn unicode_matches_case_insensitively() {
    assert!(passes("ÄPFEL", "äpfel-1"));
    assert!(passes("é", "café"));
    assert!(passes("^日本", "日本-pod"));
    assert!(passes("-f 日本", "日-x-本"));
    let parts = parts("-f ñ");
    assert!(parts.filter.pattern.unwrap().matches("PEÑA"));
}
