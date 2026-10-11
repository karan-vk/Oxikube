//! [`Buffer`]: the text completion works on, parsed only if it must be.

use std::sync::{Arc, OnceLock};

use crate::yaml::{ParseResult, parse_shared};

/// A buffer's text and, once somebody needs it, its spanned model.
///
/// Completion reads the line under the cursor lexically and usually parses a patched copy of
/// the text instead (the line is half-typed), so the model of the text as it is may never be
/// needed. Hand it a model you already hold ([`Buffer::parsed`]) and it is used; otherwise it is
/// built on first use ([`Buffer::parse`]) and available to keep ([`Buffer::parse_if_done`]).
#[derive(Debug)]
pub struct Buffer {
    text: Arc<str>,
    parsed: OnceLock<Arc<ParseResult>>,
}

impl Buffer {
    /// A buffer of `text`, not parsed yet.
    #[must_use]
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            parsed: OnceLock::new(),
        }
    }

    /// A buffer whose model is already known.
    #[must_use]
    pub fn parsed(parsed: Arc<ParseResult>) -> Self {
        Self {
            text: Arc::from(parsed.text()),
            parsed: OnceLock::from(parsed),
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The spanned model of the text, parsed on first use.
    #[must_use]
    pub fn parse(&self) -> &Arc<ParseResult> {
        self.parsed
            .get_or_init(|| Arc::new(parse_shared(self.text.clone())))
    }

    /// The model, if it has been built (so a caller can keep it for the next request).
    #[must_use]
    pub fn parse_if_done(&self) -> Option<&Arc<ParseResult>> {
        self.parsed.get()
    }
}
