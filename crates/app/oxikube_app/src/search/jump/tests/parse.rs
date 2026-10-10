//! The grammar: every form parses, every mistake says where.

use super::super::{HistoryStep, JumpCommand, ParseErrorKind, ResourceJump, parse};

fn resource(line: &str) -> ResourceJump {
    match parse(line) {
        Ok(JumpCommand::Resource(resource)) => resource,
        other => panic!("`{line}` should be a resource, got {other:?}"),
    }
}

fn error(line: &str) -> super::super::ParseError {
    parse(line).expect_err(line)
}

/// The text of `line` the error points at.
fn underlined<'a>(line: &'a str, error: &super::super::ParseError) -> &'a str {
    &line[error.span.range()]
}

#[test]
fn a_bare_alias_is_a_resource_list() {
    let jump = resource("pods");
    assert_eq!(jump.alias.value, "pods");
    assert!(jump.namespace.is_none() && jump.filter.is_none());
    assert!(jump.labels.is_none() && jump.context.is_none());
}

#[test]
fn a_bare_word_after_the_alias_is_the_namespace() {
    let jump = resource("deploy kube-system");
    assert_eq!(jump.alias.value, "deploy");
    assert_eq!(jump.namespace.unwrap().value, "kube-system");
}

#[test]
fn a_slash_word_is_the_filter() {
    let jump = resource("pod /re");
    assert_eq!(jump.filter.unwrap().value.as_str(), "re");
    assert!(jump.namespace.is_none());
}

#[test]
fn filter_forms_are_kept_as_typed_for_the_filter_bar_to_read() {
    for (line, text) in [
        ("pod /!re", "!re"),
        ("pod /^api-[0-9]+$", "^api-[0-9]+$"),
        ("pod /-l app=x", "-l app=x"),
        ("pod /-f fz", "-f fz"),
        ("pod /!-f fz", "!-f fz"),
        ("pod /", ""),
    ] {
        assert_eq!(
            resource(line).filter.unwrap().value.as_str(),
            text,
            "{line}"
        );
    }
}

#[test]
fn the_operand_of_a_filter_flag_is_not_a_selector_or_a_namespace() {
    let jump = resource("pod /-l app=x web");
    assert_eq!(jump.filter.unwrap().value.as_str(), "-l app=x");
    assert_eq!(jump.namespace.unwrap().value, "web");
    assert!(jump.labels.is_none());
}

#[test]
fn a_word_with_an_equals_sign_is_the_label_selector() {
    assert_eq!(resource("pod app=nginx").labels.unwrap().value, "app=nginx");
    assert_eq!(
        resource("pod app=x,env=y").labels.unwrap().value,
        "app=x,env=y"
    );
    assert_eq!(
        resource("pod tier!=db,app==x").labels.unwrap().value,
        "tier!=db,app==x"
    );
}

#[test]
fn an_at_word_is_the_context() {
    assert_eq!(resource("pod @prod").context.unwrap().value, "prod");
}

#[test]
fn every_token_together_in_the_documented_order() {
    let jump = resource("deploy kube-system /api app=x @prod");
    assert_eq!(jump.alias.value, "deploy");
    assert_eq!(jump.namespace.unwrap().value, "kube-system");
    assert_eq!(jump.filter.unwrap().value.as_str(), "api");
    assert_eq!(jump.labels.unwrap().value, "app=x");
    assert_eq!(jump.context.unwrap().value, "prod");
}

#[test]
fn the_tokens_after_the_alias_may_come_in_any_order() {
    assert_eq!(
        parse("deploy kube-system /api app=x @prod"),
        parse("deploy @prod app=x /api kube-system")
    );
}

#[test]
fn ctx_takes_an_optional_name() {
    assert!(matches!(parse("ctx"), Ok(JumpCommand::Ctx(None))));
    let Ok(JumpCommand::Ctx(Some(name))) = parse("ctx prod") else {
        panic!("ctx prod");
    };
    assert_eq!(name.value, "prod");
    assert!(matches!(parse("context"), Ok(JumpCommand::Ctx(None))));
}

#[test]
fn ns_takes_an_optional_name() {
    assert!(matches!(parse("ns"), Ok(JumpCommand::Ns(None))));
    let Ok(JumpCommand::Ns(Some(name))) = parse("ns kube-system") else {
        panic!("ns kube-system");
    };
    assert_eq!(name.value, "kube-system");
}

#[test]
fn q_and_quit_quit() {
    assert!(matches!(parse("q"), Ok(JumpCommand::Quit)));
    assert!(matches!(parse("quit"), Ok(JumpCommand::Quit)));
}

#[test]
fn the_history_words() {
    for (line, step) in [
        ("-", HistoryStep::Last),
        ("[", HistoryStep::Back),
        ("]", HistoryStep::Forward),
    ] {
        assert_eq!(parse(line), Ok(JumpCommand::History(step)), "{line}");
    }
}

#[test]
fn reserved_words_ignore_case_and_a_leading_colon_is_skipped() {
    assert!(matches!(parse("Q"), Ok(JumpCommand::Quit)));
    assert!(matches!(parse("CTX"), Ok(JumpCommand::Ctx(None))));
    assert_eq!(parse(":pods"), parse("pods"));
    assert_eq!(
        parse("  :  pods   kube-system  "),
        parse("pods kube-system")
    );
}

#[test]
fn spans_point_into_the_typed_line() {
    let line = ":deploy kube-system /api app=x @prod";
    let jump = resource(line);
    assert_eq!(&line[jump.alias.span.range()], "deploy");
    assert_eq!(&line[jump.namespace.unwrap().span.range()], "kube-system");
    assert_eq!(&line[jump.filter.unwrap().span.range()], "/api");
    assert_eq!(&line[jump.labels.unwrap().span.range()], "app=x");
    assert_eq!(&line[jump.context.unwrap().span.range()], "@prod");
}

#[test]
fn a_filter_with_its_operand_spans_both_words() {
    let line = "pod /-l app=x";
    let jump = resource(line);
    assert_eq!(&line[jump.filter.unwrap().span.range()], "/-l app=x");
}

#[test]
fn nothing_typed_is_an_error_at_the_end() {
    for line in ["", "   ", ":", " : "] {
        let e = error(line);
        assert_eq!(e.kind, ParseErrorKind::Empty, "{line:?}");
        assert_eq!(e.span.start, line.len());
    }
}

#[test]
fn two_filters_underline_the_second() {
    let line = "pod /api /web";
    let e = error(line);
    assert_eq!(e.kind, ParseErrorKind::DuplicateFilter);
    assert_eq!(underlined(line, &e), "/web");
}

#[test]
fn two_selectors_namespaces_and_contexts_underline_the_second() {
    for (line, kind, text) in [
        ("pod a=b c=d", ParseErrorKind::DuplicateLabels, "c=d"),
        ("pod web api", ParseErrorKind::DuplicateNamespace, "api"),
        ("pod @a @b", ParseErrorKind::DuplicateContext, "@b"),
    ] {
        let e = error(line);
        assert_eq!(e.kind, kind, "{line}");
        assert_eq!(underlined(line, &e), text, "{line}");
    }
}

#[test]
fn a_trailing_at_asks_for_the_context() {
    for line in ["pod @", "pod web @"] {
        let e = error(line);
        assert_eq!(e.kind, ParseErrorKind::MissingContext, "{line}");
        assert_eq!(underlined(line, &e), "@");
    }
}

#[test]
fn an_empty_selector_term_is_an_error() {
    for (line, text) in [
        ("pod =", "="),
        ("pod =x", "=x"),
        ("pod app=x,", "app=x,"),
        ("pod ,app=x", ",app=x"),
        ("pod a=b,,c=d", "a=b,,c=d"),
        ("pod !=x", "!=x"),
    ] {
        let e = error(line);
        assert_eq!(e.kind, ParseErrorKind::BadSelector, "{line}");
        assert!(!e.message.is_empty());
        assert!(
            underlined(line, &e).len() <= text.len() && text.contains(underlined(line, &e)),
            "{line}: underlined {:?}",
            underlined(line, &e)
        );
    }
}

#[test]
fn a_line_must_start_with_a_resource() {
    for line in ["/api", "@prod", "app=x"] {
        assert_eq!(error(line).kind, ParseErrorKind::ExpectedResource, "{line}");
    }
}

#[test]
fn reserved_words_take_no_stray_arguments() {
    for (line, text) in [
        ("q now", "now"),
        ("- x", "x"),
        ("ctx a b", "b"),
        ("ns a b", "b"),
        ("ctx @a", "@a"),
        ("ns /x", "/x"),
        ("ctx a=b", "a=b"),
    ] {
        let e = error(line);
        assert_eq!(e.kind, ParseErrorKind::Unexpected, "{line}");
        assert_eq!(underlined(line, &e), text, "{line}");
    }
}

#[test]
fn a_namespace_must_look_like_one() {
    for line in ["pod Web", "pod -x", "pod a_b", "ns Web"] {
        assert_eq!(error(line).kind, ParseErrorKind::BadNamespace, "{line}");
    }
}

#[test]
fn a_cluster_scoped_word_typed_as_a_namespace_is_still_a_namespace_to_the_parser() {
    // The parser knows no cluster; the planner decides what the namespace means.
    assert_eq!(
        resource("nodes kube-system").namespace.unwrap().value,
        "kube-system"
    );
}
