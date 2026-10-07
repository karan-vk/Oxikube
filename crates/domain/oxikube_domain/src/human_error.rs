//! [`HumanError`]: a failure as the user reads it, one plain sentence, plus the raw text behind it.
//!
//! Errors reach the screen from three places that used to word them on their own: the connect
//! view (a session's failure reason, a rendered [`OxiError`] with its `internal error:` label), the
//! log viewer and the terminal. Every one of them shows a `HumanError` the same way: the
//! [`summary`](HumanError::summary) first, the [`raw`](HumanError::raw) text behind a "Details"
//! toggle, never both at once.
//!
//! The summary is chosen from what the error says (a refused connection, an unknown host, a
//! certificate, a deadline) and else from its [`ErrorKind`]. The raw text is the adapter's
//! message, redacted, with the kind's label taken off: the kind is the summary's business.

use std::borrow::Cow;

use crate::error::{ErrorKind, OxiError};
use crate::redact::redact;

/// The longest summary taken from a message that names no kind, in characters.
const PLAIN_SUMMARY_MAX: usize = 160;

/// An error worded for a person. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanError {
    kind: Option<ErrorKind>,
    summary: String,
    raw: String,
}

impl HumanError {
    /// The error of `kind` with the adapter's `message`.
    pub fn new(kind: ErrorKind, message: &str) -> Self {
        let raw = clean(message);
        Self {
            kind: Some(kind),
            summary: summary_of(Some(kind), &raw),
            raw,
        }
    }

    /// The error `error` says.
    pub fn from_error(error: &OxiError) -> Self {
        Self::new(error.kind(), error.message())
    }

    /// An error that crossed a boundary as text: an [`OxiError`]'s `Display` (`"<label>:
    /// <message>"`, see [`ErrorKind::from_label`]) or any other message. The labels are taken
    /// off the raw text, and the first one names the kind.
    pub fn from_display(text: &str) -> Self {
        let mut rest = text.trim();
        let mut kind = None;
        while let Some((label, tail)) = rest.split_once(": ")
            && let Some(found) = ErrorKind::from_label(label.trim())
        {
            kind.get_or_insert(found);
            rest = tail.trim_start();
        }
        let raw = clean(rest);
        Self {
            kind,
            summary: summary_of(kind, &raw),
            raw,
        }
    }

    /// The same error with `summary` as its sentence: for a view that knows more than the kind
    /// does (a log stream's "pod" is a pod).
    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// What kind of failure it is, when the text named one.
    pub fn kind(&self) -> Option<ErrorKind> {
        self.kind
    }

    /// Whether the kind says the thing asked for is gone, so that trying again cannot help.
    pub fn is_not_found(&self) -> bool {
        self.kind == Some(ErrorKind::NotFound)
    }

    /// The one-sentence summary: plain words, no `internal error:` label, no addresses.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// The raw message, redacted, without the kind's label: what the Details toggle shows.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Whether the raw text says more than the summary does (there is something to show behind
    /// a Details toggle).
    pub fn has_details(&self) -> bool {
        !self.raw.is_empty() && self.raw != self.summary
    }
}

/// `text` redacted and trimmed.
fn clean(text: &str) -> String {
    redact(text).trim().to_owned()
}

/// The sentence for an error of `kind` (when known) saying `raw`.
fn summary_of(kind: Option<ErrorKind>, raw: &str) -> String {
    if let Some(known) = by_message(raw) {
        return known.to_owned();
    }
    match kind {
        Some(kind) => by_kind(kind).to_owned(),
        None => plain(raw),
    }
}

/// What the message itself says, for the failures that have a well-known cause. Checked before
/// the kind: an adapter that could not classify an error calls it internal, but a refused
/// connection is a refused connection.
fn by_message(raw: &str) -> Option<&'static str> {
    let text: Cow<str> = Cow::Owned(raw.to_ascii_lowercase());
    let has = |needle: &str| text.contains(needle);
    if has("connection refused") {
        Some("The cluster's API server refused the connection.")
    } else if has("no such host") || has("name resolution") || has("failed to lookup address") {
        Some("The API server's address could not be found.")
    } else if has("no route to host") || has("network is unreachable") {
        Some("The cluster could not be reached over the network.")
    } else if has("x509") || has("certificate") {
        Some("The cluster's certificate could not be verified.")
    } else if has("i/o timeout") || has("timed out") || has("deadline exceeded") {
        Some("The cluster did not answer in time.")
    } else {
        None
    }
}

/// The default sentence of a kind.
fn by_kind(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Auth => "The cluster rejected your credentials.",
        ErrorKind::Forbidden => "You do not have permission to do that.",
        ErrorKind::NotFound => "It was not found. It may have been deleted.",
        ErrorKind::Conflict => "The cluster changed while this was happening.",
        ErrorKind::Network => "The cluster could not be reached.",
        ErrorKind::Timeout => "The cluster did not answer in time.",
        ErrorKind::Validation => "The cluster did not accept the request.",
        ErrorKind::Unsupported => "The cluster does not support this.",
        ErrorKind::Internal => "Something went wrong inside Oxikube.",
        ErrorKind::BudgetExceeded => "Too many live views are open for this cluster.",
    }
}

/// A message that names no kind: its first line, cut on a character boundary.
fn plain(raw: &str) -> String {
    let first = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if first.chars().count() <= PLAIN_SUMMARY_MAX {
        return first.to_owned();
    }
    let mut cut: String = first.chars().take(PLAIN_SUMMARY_MAX - 1).collect();
    while cut.ends_with(char::is_whitespace) {
        cut.pop();
    }
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rendered_error_loses_its_label_and_keeps_its_kind() {
        let error = HumanError::from_display(
            "internal error: dial tcp 10.0.0.12:6443: connect: connection refused",
        );
        assert_eq!(error.kind(), Some(ErrorKind::Internal));
        assert_eq!(
            error.summary(),
            "The cluster's API server refused the connection."
        );
        assert_eq!(
            error.raw(),
            "dial tcp 10.0.0.12:6443: connect: connection refused"
        );
        assert!(error.has_details());
        assert!(!error.summary().contains("internal error"));
    }

    #[test]
    fn stacked_labels_are_all_taken_off() {
        let error = HumanError::from_display("network error: timed out: slow");
        assert_eq!(error.kind(), Some(ErrorKind::Network));
        assert_eq!(error.raw(), "slow");
    }

    #[test]
    fn the_kind_picks_the_sentence_when_the_message_names_no_cause() {
        for kind in ErrorKind::ALL {
            let error = HumanError::new(kind, "odd");
            assert_eq!(error.kind(), Some(kind));
            assert!(
                error.summary().ends_with('.'),
                "{kind}: {}",
                error.summary()
            );
            assert!(
                !error.summary().contains(':'),
                "{kind}: {}",
                error.summary()
            );
        }
        assert!(HumanError::new(ErrorKind::NotFound, "pods \"x\" not found").is_not_found());
        assert!(!HumanError::new(ErrorKind::Network, "x").is_not_found());
    }

    #[test]
    fn well_known_causes_beat_the_kind() {
        let cases = [
            (
                "no such host",
                "The API server's address could not be found.",
            ),
            (
                "x509: certificate signed by unknown authority",
                "The cluster's certificate could not be verified.",
            ),
            (
                "dial tcp: i/o timeout",
                "The cluster did not answer in time.",
            ),
            (
                "connect: no route to host",
                "The cluster could not be reached over the network.",
            ),
        ];
        for (message, summary) in cases {
            assert_eq!(
                HumanError::new(ErrorKind::Internal, message).summary(),
                summary
            );
        }
    }

    #[test]
    fn unlabelled_text_is_its_own_first_line() {
        let error = HumanError::from_display("token expired: run `aws sso login`\nmore");
        assert_eq!(error.kind(), None);
        assert_eq!(error.summary(), "token expired: run `aws sso login`");
        assert!(error.has_details(), "the second line is behind the toggle");
        let one = HumanError::from_display("plain");
        assert!(!one.has_details());
        let long = HumanError::from_display(&"é".repeat(400));
        assert_eq!(long.summary().chars().count(), PLAIN_SUMMARY_MAX);
        assert!(long.summary().ends_with('…'));
    }

    #[test]
    fn secrets_never_survive() {
        let error = HumanError::from_display(
            "internal error: Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
        );
        assert!(
            !error.raw().contains("abcdefghijklmnopqrstuvwxyz"),
            "{}",
            error.raw()
        );
        assert!(!error.summary().contains("abcdefghijklmnop"));
    }

    #[test]
    fn with_summary_replaces_only_the_sentence() {
        let error = HumanError::new(ErrorKind::NotFound, "pods \"web\" not found")
            .with_summary("The pod no longer exists.");
        assert_eq!(error.summary(), "The pod no longer exists.");
        assert_eq!(error.raw(), "pods \"web\" not found");
    }

    #[test]
    fn labels_round_trip() {
        for kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::from_label(&kind.to_string()), Some(kind));
        }
        assert_eq!(ErrorKind::from_label("nope"), None);
    }
}
