//! `:pod /re` hands the filter bar's grammar the string it would have been typed: both entry
//! points agree because they call the same `Filter::parse`.

use oxikube_domain::command::Command;

use super::super::{JumpCommand, parse as parse_jump, plan};
use super::Env;
use crate::search::filter::{FilterState, parse};

/// The text `table::SetFilter` carries for `line`.
fn sent(line: &str) -> String {
    let planned = plan(line, &Env::new()).unwrap_or_else(|e| panic!("`{line}`: {e}"));
    planned
        .commands
        .into_iter()
        .find_map(|command| match command {
            Command::TableSetFilter { text, .. } => Some(text),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`{line}` sets a filter"))
}

#[test]
fn the_jump_bar_and_the_filter_bar_parse_the_same_string() {
    for filter in [
        "web",
        "!web",
        "-l app=x",
        "-l tier=a,env!=b",
        "-f wb",
        "!-f wb",
        "web.*-1",
        r"\-x",
        "a|b",
    ] {
        let line = format!("pod /{filter}");
        let JumpCommand::Resource(jump) = parse_jump(&line).expect("parses") else {
            panic!("`{line}` is a resource jump");
        };
        let raw = jump.filter.as_ref().expect("a filter").value.as_str();
        let text = sent(&line);
        assert_eq!(text, raw, "`{line}`: the raw filter travels unchanged");
        assert_eq!(text, filter);
        // What the bar does with it is what typing `/` and the same text does.
        assert_eq!(
            parse(&text).expect("the bar parses it"),
            parse(&format!("/{filter}")).expect("typed with a slash"),
            "`{line}`"
        );
        assert!(FilterState::new().edit(&text), "`{line}`");
    }
}

#[test]
fn a_selector_word_joins_the_filter_string_the_bar_parses() {
    let both = sent("pod /web app=x");
    assert_eq!(both, "web -l app=x");
    let expr = parse(&both).expect("the bar parses it");
    let parts = expr.parts();
    assert!(parts.filter.pattern.is_some());
    assert!(parts.selector.is_some());
}

#[test]
fn a_bad_filter_travels_and_is_reported_where_it_can_be_fixed() {
    // The jump bar does not interpret the filter; the filter bar reports the error.
    let text = sent("pod /web(");
    assert_eq!(text, "web(");
    assert!(parse(&text).is_err());
}
