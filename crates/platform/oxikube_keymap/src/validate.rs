//! The keymap validator: `keymap.json` text in, positioned diagnostics out.
//!
//! One pure function, [`validate`], with no GPUI app, window or I/O, so it is the same code for
//! every caller:
//! - the store's load path ([`KeymapStore`](crate::KeymapStore)) reports its result as the
//!   notification when `keymap.json` is saved,
//! - the manifest editor runs it on a debounce on the background executor and underlines the
//!   result while `keymap.json` is open.
//!
//! It checks, per the file format in [`crate::file`]:
//! - the text is JSON with comments holding a list of sections ([`KeymapProblem::InvalidFile`],
//!   [`KeymapProblem::InvalidSection`]),
//! - each `context` parses as a key-context predicate ([`KeymapProblem::InvalidContext`]) and
//!   names only known contexts and flags ([`KeymapProblem::UnknownContext`], a warning with the
//!   nearest known name),
//! - each binding key is a sequence of valid keystrokes ([`KeymapProblem::InvalidKeystrokes`])
//!   and is not written twice in a section ([`KeymapProblem::DuplicateKeystrokes`]),
//! - each value is `null`, an action name or `[name, data]` ([`KeymapProblem::InvalidBinding`])
//!   and the action is registered ([`KeymapProblem::UnknownAction`], with the nearest name).
//!
//! What it cannot check without an app is whether an action accepts the data given for it
//! ([`KeymapProblem::InvalidActionData`]); the store adds that from the build of the bindings.
//! Every diagnostic carries its 1-based line and column and the byte span of the offending
//! token, a [`Severity`](crate::diagnostics::Severity) and a stable code.

use std::collections::BTreeSet;

use gpui::{KeyBindingContextPredicate, Keystroke};

use crate::diagnostics::{KeymapDiagnostic, KeymapProblem};
use crate::file::{KeymapAction, parse_keymap};
use crate::known::KnownNames;
use crate::layer::KeymapLayer;
use crate::lines::SourceLines;
use crate::registry::ActionRegistry;
use crate::suggest::nearest;

/// Check the user's `keymap.json` `text` against the registered `actions` and the `known`
/// context names. The result is in file order; blank text has no problems.
pub fn validate(text: &str, actions: &ActionRegistry, known: &KnownNames) -> Vec<KeymapDiagnostic> {
    let layer = KeymapLayer::User;
    let parsed = match parse_keymap(text, layer) {
        Ok(parsed) => parsed,
        Err(diagnostic) => return vec![diagnostic],
    };
    let mut found = parsed.diagnostics;
    for (index, section) in &parsed.sections {
        let mut push = |mut diagnostic: KeymapDiagnostic| {
            diagnostic.locate(&parsed.lines);
            found.push(diagnostic);
        };
        if let Some(expression) = section.context_expr() {
            for problem in check_context(expression, known) {
                push(KeymapDiagnostic::section(layer, *index, problem));
            }
        }
        for (keystrokes, value) in &section.bindings {
            for problem in check_binding(keystrokes, value, actions) {
                push(KeymapDiagnostic::binding(
                    layer, *index, keystrokes, problem,
                ));
            }
        }
    }
    for duplicate in parsed.lines.duplicates() {
        let mut diagnostic = KeymapDiagnostic::binding(
            layer,
            duplicate.section,
            &duplicate.keystrokes,
            KeymapProblem::DuplicateKeystrokes {
                keystrokes: duplicate.keystrokes.clone(),
                first_line: duplicate.first.line,
            },
        );
        diagnostic.place(&duplicate.second);
        found.push(diagnostic);
    }
    sort_in_file_order(&mut found);
    found
}

/// Order diagnostics by where they are in the text (stable, so one token's stay in check order).
pub(crate) fn sort_in_file_order(found: &mut [KeymapDiagnostic]) {
    found.sort_by_key(|diagnostic| {
        diagnostic
            .span
            .as_ref()
            .map_or(usize::MAX, |span| span.start)
    });
}

fn check_context(expression: &str, known: &KnownNames) -> Vec<KeymapProblem> {
    let predicate = match KeyBindingContextPredicate::parse(expression) {
        Ok(predicate) => predicate,
        Err(err) => {
            return vec![KeymapProblem::InvalidContext {
                context: expression.to_owned(),
                message: err.to_string(),
            }];
        }
    };
    let mut names = BTreeSet::new();
    collect_names(&predicate, &mut names);
    names
        .into_iter()
        .filter(|name| !known.contains(name))
        .map(|name| KeymapProblem::UnknownContext {
            suggestion: nearest(&name, known.all()).map(str::to_owned),
            name,
        })
        .collect()
}

/// The identifiers and value keys a predicate tests (the values of `key == value` are free text).
fn collect_names(predicate: &KeyBindingContextPredicate, out: &mut BTreeSet<String>) {
    match predicate {
        KeyBindingContextPredicate::Identifier(name) => {
            out.insert(name.to_string());
        }
        KeyBindingContextPredicate::Equal(key, _)
        | KeyBindingContextPredicate::NotEqual(key, _) => {
            out.insert(key.to_string());
        }
        KeyBindingContextPredicate::Not(inner) => collect_names(inner, out),
        KeyBindingContextPredicate::Descendant(a, b)
        | KeyBindingContextPredicate::And(a, b)
        | KeyBindingContextPredicate::Or(a, b) => {
            collect_names(a, out);
            collect_names(b, out);
        }
    }
}

fn check_binding(
    keystrokes: &str,
    value: &serde_json::Value,
    actions: &ActionRegistry,
) -> Vec<KeymapProblem> {
    let mut problems = Vec::new();
    if keystrokes.split_whitespace().next().is_none() {
        problems.push(KeymapProblem::InvalidKeystrokes {
            message: "a binding needs at least one keystroke".to_owned(),
        });
    }
    for keystroke in keystrokes.split_whitespace() {
        if let Err(err) = Keystroke::parse(keystroke) {
            problems.push(KeymapProblem::InvalidKeystrokes {
                message: err.to_string(),
            });
        }
    }
    match KeymapAction::from_json(value) {
        Err(message) => problems.push(KeymapProblem::InvalidBinding { message }),
        Ok(KeymapAction::Unbind) => {}
        Ok(KeymapAction::Action { name, .. }) => {
            if !actions.contains(&name) {
                problems.push(KeymapProblem::UnknownAction {
                    suggestion: nearest(&name, actions.names()).map(str::to_owned),
                    name,
                });
            }
        }
    }
    problems
}

/// Where in `text` a diagnostic points, for callers that only have a line: the byte offset of
/// its line and column, or the start of its line.
pub fn offset_of(text: &str, diagnostic: &KeymapDiagnostic) -> usize {
    match (&diagnostic.span, diagnostic.line) {
        (Some(span), _) => span.start,
        (None, Some(line)) => SourceLines::offset(text, line, diagnostic.column.unwrap_or(1)),
        (None, None) => 0,
    }
}
