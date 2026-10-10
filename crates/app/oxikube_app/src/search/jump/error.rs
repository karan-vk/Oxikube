//! [`ParseError`]: why a line is not a jump command, and where, so the bar can underline it and
//! offer a fix.

use std::sync::Arc;

use super::span::Span;

/// What kind of problem a [`ParseError`] is. The bar styles them alike; tests and callers (an
/// agent reading the error) match on this rather than on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// Nothing typed.
    Empty,
    /// The line starts with something that is not a resource name (`/x`, `@x`, `a=b`).
    ExpectedResource,
    /// A token the command does not take (a second word after `:q`, a stray option).
    Unexpected,
    /// A second `/filter`.
    DuplicateFilter,
    /// A second `k=v` selector.
    DuplicateLabels,
    /// A second namespace.
    DuplicateNamespace,
    /// A second `@context`.
    DuplicateContext,
    /// `@` with no context after it.
    MissingContext,
    /// A `/` filter operand missing (`/-l` and nothing after it) is not an error: the filter bar
    /// reports it. This is the label selector `k=v,..` that does not parse (an empty term, no key).
    BadSelector,
    /// A namespace that cannot be a namespace name.
    BadNamespace,
    /// No alias, resource or command has this name.
    UnknownAlias,
    /// The cluster has no such namespace.
    UnknownNamespace,
    /// No cluster context has this name.
    UnknownContext,
    /// A name that matches several contexts.
    AmbiguousContext,
    /// No cluster tab is open to jump in.
    NoCluster,
    /// The name leads to a type the cluster does not serve.
    NotServed,
    /// A user alias that expands into itself.
    AliasLoop,
}

/// A line that is not a jump command, with where the problem is.
///
/// `suggestions` are names that would fix an unknown alias, namespace or context, best first.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    /// The part of the line to underline.
    pub span: Span,
    /// The kind of problem.
    pub kind: ParseErrorKind,
    /// What is wrong, in a sentence the bar shows as is.
    pub message: String,
    /// Close names, best first (empty when there is nothing to suggest).
    pub suggestions: Vec<Arc<str>>,
}

impl ParseError {
    /// An error without suggestions.
    pub fn new(kind: ParseErrorKind, span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            kind,
            message: message.into(),
            suggestions: Vec::new(),
        }
    }

    /// Adds suggestions.
    #[must_use]
    pub fn with_suggestions(mut self, suggestions: Vec<Arc<str>>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// The same error pointing at `span` (an expanded alias reports at the word the user typed).
    #[must_use]
    pub fn at(mut self, span: Span) -> Self {
        self.span = span;
        self
    }
}
