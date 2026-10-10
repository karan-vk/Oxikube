//! Each syntax form, escaping, empty input, invalid regex, unicode.

use super::*;
use crate::store::filter::{FilterError, TextPattern};
use crate::store::{LabelSelector, LabelTerm};

#[test]
fn empty_and_pending_inputs_are_no_filter() {
    for input in ["", "   ", "/", " / ", "-", "/-", "-f", "-f  ", "-l", "/-l "] {
        assert_eq!(ok(input).parts(), FilterParts::default(), "{input:?}");
        assert!(parts(input).is_empty(), "{input:?}");
    }
}

#[test]
fn a_slash_alone_clears_the_filter() {
    assert_eq!(ok("/"), FilterExpr::Empty);
    assert_eq!(ok("/"), ok(""));
}

#[test]
fn a_bang_with_no_pattern_is_an_error() {
    for input in ["!", "/!", "! ", "!-f", "/!-f", "!-f  ", "! -f", "!-"] {
        assert_eq!(
            parse(input),
            Err(FilterError::NothingToInvert),
            "{input:?}: nothing to invert"
        );
    }
    let message = FilterError::NothingToInvert.to_string();
    assert!(message.contains("pattern"), "{message}");
}

#[test]
fn a_pattern_with_spaces_is_kept_whole() {
    // Names have no spaces, but a regex may (`a b` never matches a name; the pattern is not split
    // into words), so the text must reach the matcher untouched.
    let FilterExpr::Text(pattern) = ok("/my app") else {
        panic!("a text pattern");
    };
    assert_eq!(pattern.as_str(), "my app");
    assert!(passes("web  x", "web  x"));
    assert!(!passes("web x", "web-x"));
}

#[test]
fn a_pattern_longer_than_the_cap_is_rejected_without_compiling() {
    use crate::store::filter::MAX_FILTER_LEN;
    let at_cap = "a".repeat(MAX_FILTER_LEN);
    assert!(parse(&at_cap).is_ok());
    for input in [
        "a".repeat(MAX_FILTER_LEN + 1),
        format!("({})", "a|".repeat(MAX_FILTER_LEN)),
        format!("-l app={}", "x".repeat(MAX_FILTER_LEN)),
        "é".repeat(MAX_FILTER_LEN + 1),
    ] {
        assert_eq!(
            parse(&input),
            Err(FilterError::TooLong(MAX_FILTER_LEN)),
            "{} chars",
            input.chars().count()
        );
    }
    assert!(
        parse(&"é".repeat(MAX_FILTER_LEN)).is_ok(),
        "the cap counts characters, not bytes"
    );
}

#[test]
fn a_pathological_pattern_is_bounded_and_never_blocks() {
    // The regex engine has no backtracking, so the classic blow-ups are linear; nested counted
    // repeats are refused by its size limit. Either way the call returns at once.
    let started = std::time::Instant::now();
    for input in ["(a*)*b", "(a|aa)+$", "(x+x+)+y", "((a{100}){100}){100}"] {
        let _ = parse(input);
    }
    let name = "a".repeat(5_000);
    assert!(!passes("(a*)*b", &name));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
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

#[test]
fn a_name_filter_and_a_selector_apply_together() {
    // What `:pod /api app=x` types into the bar.
    let p = parts("api -l app=x");
    assert!(
        p.filter
            .pattern
            .as_ref()
            .is_some_and(|n| n.matches("api-1"))
    );
    assert!(
        p.filter
            .pattern
            .as_ref()
            .is_some_and(|n| !n.matches("web-1"))
    );
    assert_eq!(
        p.selector,
        Some(LabelSelector::from_terms(vec![LabelTerm::Eq(
            "app".into(),
            "x".into()
        )]))
    );
    assert_eq!(parts("/api -l app=x"), p, "a leading slash is ignored");

    let inverse = parts("!api -l app=x");
    assert!(
        inverse
            .filter
            .pattern
            .as_ref()
            .is_some_and(|n| n.matches("web-1"))
    );
    assert!(inverse.selector.is_some());

    let fuzzy = parts("-f ap -l app=x,env=y");
    assert!(fuzzy.ranks(), "a fuzzy name filter still ranks");
    assert_eq!(fuzzy.selector.map(|s| s.terms().len()), Some(2));
}

#[test]
fn a_selector_after_a_name_with_nothing_else_is_just_the_other_part() {
    assert_eq!(ok("web -l"), ok("web"));
    assert_eq!(ok("web -l   "), ok("web"));
    assert_eq!(
        parts("-l app=x").filter.pattern,
        None,
        "a selector alone has no name filter"
    );
}

#[test]
fn a_bad_selector_after_a_name_is_reported() {
    assert!(matches!(
        parse("web -l zone in ("),
        Err(FilterError::Selector(_))
    ));
    assert!(matches!(parse("web(-l app=x"), Err(FilterError::Regex(_))));
}
