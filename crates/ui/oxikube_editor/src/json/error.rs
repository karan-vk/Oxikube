//! [`ConvertError`]: why a buffer could not be converted.

use std::fmt;
use std::ops::Range;

/// A conversion that was refused. The buffer is left as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertError {
    /// What is wrong and what to do about it, in one line.
    pub message: String,
    /// The bytes of the source to show the user, when the problem has a place in the text.
    pub span: Option<Range<usize>>,
}

impl ConvertError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }

    pub(super) fn at(message: impl Into<String>, span: Range<usize>) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
        }
    }

    /// Prefixes the message with the 1-based line of the span's start in `text`.
    #[must_use]
    pub(super) fn with_line(mut self, text: &str) -> Self {
        if let Some(span) = &self.span {
            let line = line_of(text, span.start);
            self.message = format!("line {line}: {}", self.message);
        }
        self
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConvertError {}

/// The 1-based line of byte `offset` in `text`.
fn line_of(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    1 + text.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}
