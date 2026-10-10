//! Completion for the bar: which word is being typed ([`site`]) and what could stand for it
//! ([`candidates`]).
//!
//! The first word offers the aliases of the cluster and the reserved words (`ctx`, `ns`, `q`);
//! the second, the namespaces of the cluster (after `ctx`, the contexts); a word after `@`, the
//! contexts. Ranking by closeness to what is typed is the picker's (`oxikube_palette`), which
//! matches off the UI thread above a few hundred candidates; this module only says what the
//! candidates are, so it is plain data and testable without a window.

use std::sync::Arc;

use super::env::JumpEnv;
use super::span::Span;
use super::token::{takes_operand, tokenize};
use crate::session::namespaces::NamespaceCatalog;

/// What kind of word is being typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The first word: a resource alias or a reserved word.
    Alias,
    /// A namespace.
    Namespace,
    /// A cluster context (after `ctx`, or after `@`).
    Context,
    /// A word with nothing to complete (a filter, a selector, after `:q`).
    Nothing,
}

/// The word under the caret (the end of the line) and what it can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionSite {
    /// What kind of word it is.
    pub slot: Slot,
    /// What is typed of it already (without a leading `@`).
    pub prefix: String,
    /// The bytes a completion replaces: the typed part of the word, after the `@` if there is
    /// one. Empty and at the end of the line when a new word has not been started.
    pub replace: Span,
}

/// Finds the word being typed at the end of `line`.
pub fn site(line: &str) -> CompletionSite {
    let tokens = tokenize(line);
    let starts_new = line.is_empty() || line.ends_with(char::is_whitespace) || tokens.is_empty();
    let (index, partial, span) = if starts_new {
        (tokens.len(), "", Span::new(line.len(), line.len()))
    } else {
        let last = tokens[tokens.len() - 1];
        (tokens.len() - 1, last.text, last.span)
    };

    let nothing = |span| CompletionSite {
        slot: Slot::Nothing,
        prefix: String::new(),
        replace: span,
    };
    let with = |slot, prefix: &str, span: Span| CompletionSite {
        slot,
        prefix: prefix.to_owned(),
        replace: span,
    };

    if index == 0 {
        return with(Slot::Alias, partial, span);
    }
    if let Some(name) = partial.strip_prefix('@') {
        return with(Slot::Context, name, Span::new(span.start + 1, span.end));
    }
    if partial.starts_with('/') || partial.contains('=') {
        return nothing(span);
    }
    let head = tokens[0].text.to_ascii_lowercase();
    match head.as_str() {
        "ctx" | "context" if index == 1 => with(Slot::Context, partial, span),
        "ns" if index == 1 => with(Slot::Namespace, partial, span),
        "q" | "quit" | "-" | "[" | "]" | "ctx" | "context" | "ns" => nothing(span),
        _ => {
            let is_flag = |text: &str| text.strip_prefix('/').is_some_and(takes_operand);
            // The word right after `/-l` is the filter's operand, not a namespace.
            if index > 1 && is_flag(tokens[index - 1].text) {
                return nothing(span);
            }
            // A bare word already typed (and not an operand) is the namespace: only one.
            let mut after_flag = false;
            let taken = tokens[1..index].iter().any(|token| {
                let operand = after_flag;
                after_flag = is_flag(token.text);
                !operand && !token.text.starts_with(['/', '@']) && !token.text.contains('=')
            });
            if taken {
                nothing(span)
            } else {
                with(Slot::Namespace, partial, span)
            }
        }
    }
}

/// One thing a word could be completed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The word to insert.
    pub text: Arc<str>,
    /// What it stands for (`apps/v1 deployments`, `connected`), shown dimmed next to it.
    pub detail: Arc<str>,
}

/// Every candidate for `slot` in the cluster `env` acts on (the line's `@ctx` is not looked at:
/// the list follows the shown cluster, which is what the user sees). Sorted by text. One call
/// builds a few hundred to a few thousand entries: make it when the bar opens or the slot
/// changes, and filter the result on each keystroke.
pub fn candidates(slot: Slot, env: &dyn JumpEnv) -> Vec<Candidate> {
    match slot {
        Slot::Alias => alias_candidates(env),
        Slot::Namespace => namespace_candidates(env),
        Slot::Context => env
            .contexts()
            .iter()
            .map(|c| Candidate {
                text: c.name.clone(),
                detail: if c.connected { "connected" } else { "" }.into(),
            })
            .collect(),
        Slot::Nothing => Vec::new(),
    }
}

const RESERVED: [(&str, &str); 3] = [
    ("ctx", "switch the cluster context"),
    ("ns", "namespaces"),
    ("q", "quit"),
];

fn alias_candidates(env: &dyn JumpEnv) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = match env.active_cluster() {
        Some(cluster) => env
            .aliases(&cluster)
            .entries()
            .into_iter()
            .map(|entry| Candidate {
                detail: entry.target.to_string().into(),
                text: entry.name,
            })
            .collect(),
        None => Vec::new(),
    };
    for (word, detail) in RESERVED {
        if !out.iter().any(|c| &*c.text == word) {
            out.push(Candidate {
                text: word.into(),
                detail: detail.into(),
            });
        }
    }
    out.sort_by(|a, b| a.text.cmp(&b.text));
    out
}

fn namespace_candidates(env: &dyn JumpEnv) -> Vec<Candidate> {
    let Some(cluster) = env.active_cluster() else {
        return Vec::new();
    };
    let Some(NamespaceCatalog { names, .. }) = env.namespaces(&cluster) else {
        return Vec::new();
    };
    let mut out = vec![Candidate {
        text: "all".into(),
        detail: "every namespace".into(),
    }];
    out.extend(
        names
            .into_iter()
            .filter(|n| n != "all")
            .map(|name| Candidate {
                text: name.into(),
                detail: "".into(),
            }),
    );
    out
}

/// The line with the word at `site` replaced by `candidate` and a space after it, ready for the
/// next word.
pub fn accept(line: &str, site: &CompletionSite, candidate: &str) -> String {
    let mut out = String::with_capacity(line.len() + candidate.len() + 1);
    out.push_str(&line[..site.replace.start]);
    out.push_str(candidate);
    out.push(' ');
    out
}
