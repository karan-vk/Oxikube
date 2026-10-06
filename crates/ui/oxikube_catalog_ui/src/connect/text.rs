//! What the connect view may show of an error: redacted, summarised, never cut mid-character.

use oxikube_domain::redact::redact;

/// The longest summary line, in characters. The rest is in the details.
pub const SUMMARY_MAX_CHARS: usize = 180;

/// Removes what must never reach the screen (tokens, `Authorization` headers, URL passwords,
/// Secret data) from `text`. Everything the connect view shows passes through here, whatever
/// produced it: the session manager redacts reasons already, this is the second lock on the
/// door (non-negotiable 5).
pub fn scrub(text: &str) -> String {
    redact(text).into_owned()
}

/// An error text split for display: a one-line summary and the full text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayText {
    /// The first non-empty line, cut to [`SUMMARY_MAX_CHARS`] characters with an ellipsis.
    pub summary: String,
    /// The whole text, redacted, trimmed.
    pub full: String,
    /// Whether `summary` is shorter than `full` (there is more to read in the details).
    pub truncated: bool,
}

impl DisplayText {
    /// Splits `raw` (redacting it first).
    pub fn new(raw: &str) -> Self {
        let full = scrub(raw).trim().to_owned();
        let first = full
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default();
        let summary = shorten(first, SUMMARY_MAX_CHARS);
        let truncated = summary != full;
        Self {
            summary,
            full,
            truncated,
        }
    }

    /// Whether there is no text at all.
    pub fn is_empty(&self) -> bool {
        self.full.is_empty()
    }
}

/// `text` cut to at most `max` characters, with `…` replacing what was cut. Cuts on a character
/// boundary, so multi-byte text never panics.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(1);
    let mut cut: String = text.chars().take(keep).collect();
    while cut.ends_with(char::is_whitespace) {
        cut.pop();
    }
    cut.push('…');
    cut
}
