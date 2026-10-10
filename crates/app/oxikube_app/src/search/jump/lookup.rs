//! Looking names up for the planner: the namespace a word selects, the context it names, and the
//! errors for a name the cluster does not have.

use std::sync::Arc;

use oxikube_domain::ids::ClusterId;

use super::env::{JumpContext, JumpEnv};
use super::error::{ParseError, ParseErrorKind};
use super::span::{Span, Spanned};
use crate::search::aliases::closest_names;

pub(super) fn no_cluster(span: Span) -> ParseError {
    ParseError::new(
        ParseErrorKind::NoCluster,
        span,
        "Open a cluster first: there is no cluster tab to jump in",
    )
}

/// The names `namespace::Select` gets for a namespace word: `all` is every namespace (an empty
/// selection) unless the cluster has a namespace of that name; a name the cluster does not have
/// is an error when the cluster's own list says so.
pub(super) fn namespace_names(
    word: &Spanned<String>,
    cluster: &ClusterId,
    env: &dyn JumpEnv,
) -> Result<Vec<String>, ParseError> {
    let name = word.value.as_str();
    let catalog = env.namespaces(cluster);
    let listed = catalog.as_ref().is_some_and(|c| c.contains(name));
    if name == "all" && !listed {
        return Ok(Vec::new());
    }
    if let Some(catalog) = &catalog
        && catalog.is_authoritative()
        && !listed
    {
        let suggestions = closest_names(name, catalog.names.iter().map(String::as_str));
        return Err(ParseError::new(
            ParseErrorKind::UnknownNamespace,
            word.span,
            unknown_message("namespace", name, &suggestions),
        )
        .with_suggestions(suggestions));
    }
    Ok(vec![name.to_owned()])
}

/// The context `word` names: the exact name, else the same ignoring case, else the only context
/// it is a prefix of.
pub(super) fn find_context<'e>(
    word: &Spanned<String>,
    env: &'e dyn JumpEnv,
) -> Result<&'e JumpContext, ParseError> {
    let name = word.value.as_str();
    let contexts = env.contexts();
    if let Some(found) = contexts.iter().find(|c| &*c.name == name) {
        return Ok(found);
    }
    let lower = name.to_ascii_lowercase();
    if let Some(found) = contexts
        .iter()
        .find(|c| c.name.to_ascii_lowercase() == lower)
    {
        return Ok(found);
    }
    let prefixed: Vec<&JumpContext> = contexts
        .iter()
        .filter(|c| c.name.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match prefixed.as_slice() {
        [only] => Ok(only),
        [] => {
            let suggestions = closest_names(name, contexts.iter().map(|c| &*c.name));
            Err(ParseError::new(
                ParseErrorKind::UnknownContext,
                word.span,
                unknown_message("context", name, &suggestions),
            )
            .with_suggestions(suggestions))
        }
        several => {
            let suggestions: Vec<_> = several.iter().take(5).map(|c| c.name.clone()).collect();
            Err(ParseError::new(
                ParseErrorKind::AmbiguousContext,
                word.span,
                format!(
                    "`{name}` matches several contexts: {}",
                    suggestions
                        .iter()
                        .map(|s| &**s)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_suggestions(suggestions))
        }
    }
}

pub(super) fn unknown_alias(alias: &Spanned<String>, suggestions: Vec<Arc<str>>) -> ParseError {
    ParseError::new(
        ParseErrorKind::UnknownAlias,
        alias.span,
        unknown_message("resource", &alias.value, &suggestions),
    )
    .with_suggestions(suggestions)
}

/// `Unknown resource `pdos`. Did you mean pods, pdb?`
fn unknown_message(what: &str, name: &str, suggestions: &[Arc<str>]) -> String {
    let mut message = format!("Unknown {what} `{name}`.");
    if !suggestions.is_empty() {
        message.push_str(" Did you mean ");
        for (i, suggestion) in suggestions.iter().take(3).enumerate() {
            if i > 0 {
                message.push_str(", ");
            }
            message.push_str(suggestion);
        }
        message.push('?');
    }
    message
}
