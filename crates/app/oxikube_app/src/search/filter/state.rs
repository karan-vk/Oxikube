//! [`FilterState`]: the text of a filter, the last good parse and the error of the text.

use super::{FilterError, FilterParts, parse};

/// What a view keeps of its `/` filter: the text typed, the parts of the last text that parsed
/// and the error of the current text, if it has one.
///
/// A small value type: the keymap actions for `/`, `escape` and the filter bar only call its
/// methods, and it holds no entity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterState {
    text: String,
    parts: FilterParts,
    error: Option<FilterError>,
}

impl FilterState {
    /// No filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes `text` as the new filter text. A text that parses replaces the filter and clears the
    /// error; one that does not keeps the last good filter and records why. Returns whether the
    /// text parsed.
    pub fn edit(&mut self, text: &str) -> bool {
        if text == self.text && self.error.is_none() {
            return true;
        }
        text.clone_into(&mut self.text);
        match parse(text) {
            Ok(expr) => {
                self.parts = expr.parts();
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Removes the filter and its error (`escape`, the chip's cross, a bare `/`).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The text as typed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The parts of the last text that parsed: what the table shows rows for.
    pub fn parts(&self) -> &FilterParts {
        &self.parts
    }

    /// Why the current text is not a filter, if it is not one.
    pub fn error(&self) -> Option<&FilterError> {
        self.error.as_ref()
    }

    /// Whether a filter is in effect (the last good parse lets something out).
    pub fn is_active(&self) -> bool {
        !self.parts.is_empty()
    }
}
