//! The parser: a jump line to a [`JumpCommand`], or the first thing wrong with it.
//!
//! The grammar, in the order tokens are read:
//!
//! ```text
//! line     = [ ":" ] ( resource | ctx | ns | quit | history )
//! resource = alias { ns | "/" filter | selector | "@" context }     (each at most once)
//! ctx      = "ctx" [ name ]        ns   = "ns" [ name ]        quit = "q" | "quit"
//! history  = "-" | "[" | "]"
//! ```
//!
//! A token starting with `/` is the filter (`/-l`, `/-f` and `/!-f` take the next word as their
//! operand: `/-l app=x`), one starting with `@` the context, one containing `=` a label selector
//! list, and any other word the namespace. The first word is the alias or a reserved word.
//! There is no quoting and no regex: a hand-written tokenizer and a loop.

use super::ast::{HistoryStep, JumpCommand, RawFilter, ResourceJump};
use super::error::{ParseError, ParseErrorKind};
use super::span::{Span, Spanned};
use super::token::{Token, takes_operand, tokenize};
use crate::session::namespaces::is_valid_namespace_name;
use crate::store::LabelSelector;

/// Parses `line` (what is typed after the `:`; a leading `:` is accepted and ignored).
///
/// Pure and allocation-light: run it on every keystroke.
///
/// # Errors
///
/// The first problem as a [`ParseError`] with the span to underline.
pub fn parse(line: &str) -> Result<JumpCommand, ParseError> {
    let tokens = tokenize(line);
    let Some(first) = tokens.first().copied() else {
        return Err(ParseError::new(
            ParseErrorKind::Empty,
            Span::new(line.len(), line.len()),
            "Type a resource (pods, deploy), `ctx`, `ns` or `q`",
        ));
    };
    let rest = &tokens[1..];

    let word = first.text.to_ascii_lowercase();
    match word.as_str() {
        "-" => history(HistoryStep::Last, rest),
        "[" => history(HistoryStep::Back, rest),
        "]" => history(HistoryStep::Forward, rest),
        "q" | "quit" => {
            no_more(rest, "`q` takes no arguments")?;
            Ok(JumpCommand::Quit)
        }
        "ctx" | "context" => Ok(JumpCommand::Ctx(optional_name(rest, "ctx")?)),
        "ns" => Ok(JumpCommand::Ns(optional_name(rest, "ns")?)),
        _ => resource(first, rest),
    }
}

fn history(step: HistoryStep, rest: &[Token<'_>]) -> Result<JumpCommand, ParseError> {
    no_more(rest, "`-`, `[` and `]` take no arguments")?;
    Ok(JumpCommand::History(step))
}

/// Errors on the first of `rest`.
fn no_more(rest: &[Token<'_>], message: &str) -> Result<(), ParseError> {
    match rest.first() {
        Some(extra) => Err(ParseError::new(
            ParseErrorKind::Unexpected,
            extra.span,
            message,
        )),
        None => Ok(()),
    }
}

/// `ctx [name]` and `ns [name]`: at most one plain word.
fn optional_name(rest: &[Token<'_>], command: &str) -> Result<Option<Spanned<String>>, ParseError> {
    let Some(name) = rest.first() else {
        return Ok(None);
    };
    if name.text.starts_with(['/', '@']) || name.text.contains('=') {
        return Err(ParseError::new(
            ParseErrorKind::Unexpected,
            name.span,
            format!("`{command}` takes a name, not `{}`", name.text),
        ));
    }
    no_more(&rest[1..], &format!("`{command}` takes one name"))?;
    if command == "ns" {
        check_namespace(name)?;
    }
    Ok(Some(Spanned::new(name.text.to_owned(), name.span)))
}

fn resource(first: Token<'_>, rest: &[Token<'_>]) -> Result<JumpCommand, ParseError> {
    if first.text.starts_with(['/', '@', ':']) || first.text.contains('=') {
        return Err(ParseError::new(
            ParseErrorKind::ExpectedResource,
            first.span,
            "Start with a resource name, like `pods` or `deploy`",
        ));
    }
    let mut jump = ResourceJump {
        alias: Spanned::new(first.text.to_owned(), first.span),
        namespace: None,
        filter: None,
        labels: None,
        context: None,
    };

    let mut i = 0;
    while i < rest.len() {
        let token = rest[i];
        i += 1;
        if let Some(text) = token.text.strip_prefix('/') {
            if jump.filter.is_some() {
                return Err(ParseError::new(
                    ParseErrorKind::DuplicateFilter,
                    token.span,
                    "Only one /filter is allowed; remove this one",
                ));
            }
            let mut text = text.to_owned();
            let mut span = token.span;
            if takes_operand(&text)
                && let Some(operand) = rest.get(i)
                && !operand.text.starts_with(['/', '@'])
            {
                text.push(' ');
                text.push_str(operand.text);
                span = span.to(operand.span);
                i += 1;
            }
            jump.filter = Some(Spanned::new(RawFilter::new(text), span));
        } else if let Some(name) = token.text.strip_prefix('@') {
            if jump.context.is_some() {
                return Err(ParseError::new(
                    ParseErrorKind::DuplicateContext,
                    token.span,
                    "Only one @context is allowed; remove this one",
                ));
            }
            if name.is_empty() {
                return Err(ParseError::new(
                    ParseErrorKind::MissingContext,
                    token.span,
                    "`@` needs a context name after it, like `@prod`",
                ));
            }
            jump.context = Some(Spanned::new(name.to_owned(), token.span));
        } else if token.text.contains('=') {
            if jump.labels.is_some() {
                return Err(ParseError::new(
                    ParseErrorKind::DuplicateLabels,
                    token.span,
                    "Only one label selector is allowed; join them with commas (app=x,env=y)",
                ));
            }
            check_selector(&token)?;
            jump.labels = Some(Spanned::new(token.text.to_owned(), token.span));
        } else {
            if jump.namespace.is_some() {
                return Err(ParseError::new(
                    ParseErrorKind::DuplicateNamespace,
                    token.span,
                    "Only one namespace is allowed; remove this one",
                ));
            }
            check_namespace(&token)?;
            jump.namespace = Some(Spanned::new(token.text.to_owned(), token.span));
        }
    }
    Ok(JumpCommand::Resource(jump))
}

fn check_namespace(token: &Token<'_>) -> Result<(), ParseError> {
    if is_valid_namespace_name(token.text) {
        return Ok(());
    }
    Err(ParseError::new(
        ParseErrorKind::BadNamespace,
        token.span,
        format!("`{}` is not a namespace name", token.text),
    ))
}

/// A `k=v,k2!=v2` token: no empty term, a key in every term, and the selector grammar of the
/// filter bar.
fn check_selector(token: &Token<'_>) -> Result<(), ParseError> {
    let mut offset = token.span.start;
    for part in token.text.split(',') {
        let span = Span::new(offset, offset + part.len());
        offset += part.len() + 1;
        if part.is_empty() {
            return Err(ParseError::new(
                ParseErrorKind::BadSelector,
                token.span,
                "Empty label selector term; write app=x,env=y",
            ));
        }
        let key = part.split(['=', '!']).next().unwrap_or_default();
        if key.is_empty() {
            return Err(ParseError::new(
                ParseErrorKind::BadSelector,
                span,
                "A label selector needs a key before `=`",
            ));
        }
    }
    LabelSelector::parse(token.text).map(drop).map_err(|error| {
        ParseError::new(ParseErrorKind::BadSelector, token.span, error.to_string())
    })
}
