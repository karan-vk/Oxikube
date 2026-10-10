//! [`Span`] and [`Spanned`]: where in the typed line a token is, so the bar can underline it.

use std::ops::Range;

/// A byte range of the line the user typed (`start..end`, on character boundaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// The range `start..end`.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The smallest span covering `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// The span as a range, for slicing the line.
    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }

    /// Whether the span covers no byte.
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// A value with the [`Span`] it was read from. Equality looks at the value only, so a line and
/// its pretty-printed form compare equal once parsed.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Where it was.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// `value` read from `span`.
    pub fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }
}

impl<T: PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq> Eq for Spanned<T> {}
