//! `LogFilter` / `LogMatcher`: regex, literal, case toggle, inverse, invalid patterns, unicode.

use crate::logs::LogFilter;

fn matcher(pattern: &str, case_sensitive: bool, inverse: bool) -> crate::logs::LogMatcher {
    LogFilter {
        pattern: pattern.to_owned(),
        case_sensitive,
        inverse,
    }
    .compile()
    .expect("a valid pattern")
}

#[test]
fn a_literal_pattern_matches_anywhere_in_the_line() {
    let m = matcher("timeout", false, false);
    assert!(m.matches("GET /x failed: timeout after 3s"));
    assert!(!m.matches("all good"));
}

#[test]
fn regex_syntax_works() {
    let m = matcher(r"status=(4|5)\d\d", false, false);
    assert!(m.matches("GET / status=503 in 3ms"));
    assert!(m.matches("status=404"));
    assert!(!m.matches("status=200"));
    let anchored = matcher("^ERROR", true, false);
    assert!(anchored.matches("ERROR boom"));
    assert!(!anchored.matches("an ERROR boom"));
}

#[test]
fn case_is_insensitive_until_toggled() {
    assert!(matcher("error", false, false).matches("An ERROR happened"));
    assert!(!matcher("error", true, false).matches("An ERROR happened"));
    assert!(matcher("ERROR", true, false).matches("An ERROR happened"));
}

#[test]
fn inverse_keeps_the_lines_without_the_pattern() {
    let m = matcher("health", false, true);
    assert!(!m.matches("GET /healthz 200"));
    assert!(m.matches("POST /orders 201"));
    // `contains` ignores the inverse: it is what is highlighted.
    assert!(m.contains("GET /healthz 200"));
}

#[test]
fn the_empty_pattern_matches_everything_inverse_or_not() {
    for inverse in [false, true] {
        let m = matcher("", false, inverse);
        assert!(!m.is_active());
        assert!(m.matches("anything"));
        assert!(m.matches(""));
        assert!(m.spans("anything", 10).is_empty());
    }
}

#[test]
fn an_invalid_pattern_is_an_error_with_a_one_line_reason() {
    let error = LogFilter::new("foo(").compile().unwrap_err();
    assert!(!error.message().contains('\n'), "{error}");
    assert!(error.message().contains("unclosed group"), "{error}");
    assert_eq!(error.to_string(), error.message());
    assert!(LogFilter::new("[a-").compile().is_err());
    assert!(LogFilter::new("*x").compile().is_err());
}

#[test]
fn a_pattern_that_compiles_to_too_large_a_program_is_refused() {
    assert!(LogFilter::new("((a{100}){100}){100}").compile().is_err());
}

#[test]
fn unicode_matches_by_character_and_spans_are_byte_ranges_on_boundaries() {
    let m = matcher("ünï", false, false);
    let text = "Übergröße ÜNÏcode ünï";
    assert!(m.matches(text));
    let spans = m.spans(text, 10);
    assert_eq!(spans.len(), 2);
    for span in &spans {
        assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
    }
    assert_eq!(&text[spans[0].clone()], "ÜNÏ");
    assert_eq!(&text[spans[1].clone()], "ünï");
    assert!(matcher("日本", true, false).matches("ログ 日本語"));
    assert!(matcher(".", true, false).matches("é"));
}

#[test]
fn spans_are_the_occurrences_never_empty_and_capped() {
    let m = matcher("ab", false, false);
    assert_eq!(m.spans("ab xx AB ab", 10), [0..2, 6..8, 9..11]);
    assert_eq!(m.spans("ab xx AB ab", 2), [0..2, 6..8]);
    // A pattern that can match nothing has no span to draw but still matches the line.
    let star = matcher("x*", false, false);
    assert!(star.matches("abc"));
    assert!(star.spans("abc", 10).is_empty());
}
