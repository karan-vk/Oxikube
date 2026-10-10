//! The canonical form of a parsed line, and the property that printing then parsing again
//! changes nothing.

use proptest::prelude::*;

use super::super::{JumpCommand, parse};

fn canonical(line: &str) -> String {
    parse(line).expect(line).to_string()
}

#[test]
fn printing_puts_the_tokens_in_the_documented_order() {
    assert_eq!(
        canonical("deploy @prod app=x /api kube-system"),
        "deploy kube-system /api app=x @prod"
    );
}

#[test]
fn reserved_words_print_in_their_short_form() {
    assert_eq!(canonical(":quit"), "q");
    assert_eq!(canonical("context prod"), "ctx prod");
    assert_eq!(canonical("ns"), "ns");
    assert_eq!(canonical("-"), "-");
    assert_eq!(canonical("["), "[");
    assert_eq!(canonical("]"), "]");
}

#[test]
fn filters_keep_their_operands() {
    assert_eq!(canonical("pod /-l app=x"), "pod /-l app=x");
    assert_eq!(canonical("pod /!-f   fz"), "pod /!-f fz");
    assert_eq!(canonical("pod /"), "pod /");
}

#[test]
fn only_navigation_is_recorded() {
    for (line, recorded) in [
        ("pods", true),
        ("ctx prod", true),
        ("ns", true),
        ("q", false),
        ("-", false),
        ("[", false),
    ] {
        assert_eq!(parse(line).unwrap().is_recorded(), recorded, "{line}");
    }
}

/// A random but valid line: an alias, then each of the four optional tokens in a random order.
fn arbitrary_line() -> impl Strategy<Value = String> {
    let alias = prop::sample::select(vec![
        "pods",
        "po",
        "deploy",
        "dp",
        "svc",
        "certs",
        "widgets.example.io",
        "Ing",
    ]);
    let namespace = prop::option::of(prop::sample::select(vec![
        "default",
        "kube-system",
        "a",
        "all",
        "team-1",
    ]));
    let filter = prop::option::of(prop::sample::select(vec![
        "/re",
        "/!re",
        "/^api-[0-9]+$",
        "/-l app=x",
        "/-f fz",
        "/!-f fz",
        "/-l tier!=db,env=prod",
        "/",
    ]));
    let labels = prop::option::of(prop::sample::select(vec![
        "app=nginx",
        "app=x,env=y",
        "tier!=db",
        "a==b",
    ]));
    let context = prop::option::of(prop::sample::select(vec!["@prod", "@dev-1", "@a.b"]));
    let order = Just(vec![0usize, 1, 2, 3]).prop_shuffle();
    (alias, namespace, filter, labels, context, order).prop_map(
        |(alias, namespace, filter, labels, context, order)| {
            let parts = [namespace, filter, labels, context];
            let mut line = alias.to_owned();
            for i in order {
                if let Some(part) = parts[i] {
                    line.push(' ');
                    line.push_str(part);
                }
            }
            line
        },
    )
}

proptest! {
    /// Parse, print, parse again: the same tree, and printing is a fixed point.
    #[test]
    fn printing_and_reparsing_is_stable(line in arbitrary_line()) {
        let first = parse(&line).expect("generated lines are valid");
        let printed = first.to_string();
        let second = parse(&printed).expect("a printed line parses");
        prop_assert_eq!(&first, &second);
        prop_assert_eq!(second.to_string(), printed);
    }

    /// Whatever the user types, the parser answers (no panic) and a line it accepts survives the
    /// round trip.
    #[test]
    fn arbitrary_text_never_panics_and_accepted_text_round_trips(line in "[ -~]{0,40}") {
        if let Ok(first) = parse(&line) {
            let again = parse(&first.to_string());
            prop_assert_eq!(again.as_ref(), Ok(&first), "{} -> {}", line, first);
        }
    }

    /// A parse error always points inside the line.
    #[test]
    fn error_spans_stay_inside_the_line(line in "[ -~]{0,40}") {
        if let Err(error) = parse(&line) {
            prop_assert!(error.span.start <= error.span.end);
            prop_assert!(error.span.end <= line.len());
            prop_assert!(line.is_char_boundary(error.span.start));
            prop_assert!(line.is_char_boundary(error.span.end));
        }
    }
}

#[test]
fn history_words_are_one_command_each() {
    assert!(matches!(parse("-"), Ok(JumpCommand::History(_))));
}
