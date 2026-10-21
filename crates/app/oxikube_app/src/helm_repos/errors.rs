//! Readable reasons for repository failures.

use oxikube_domain::{ErrorKind, OxiError};

/// One short sentence for `error`: what went wrong and, where it helps, what to do.
///
/// `helm` reports an unreachable URL, a 401, an unparsable index and a locked repository file in
/// its stderr; the redacted summary is classified here and the original text kept after it.
pub fn describe_error(error: &OxiError) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();
    let known = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    // An HTTP status is a number on its own: not part of a port (`:8401`) or a longer number.
    let status = |code: &str| {
        lower.match_indices(code).any(|(at, _)| {
            let before = lower[..at].chars().next_back();
            let after = lower[at + code.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == ':' || c == '.' || c == '-')
                && !after.is_some_and(|c| c.is_alphanumeric())
        })
    };
    let reason = if error.kind() == ErrorKind::Unsupported {
        return text;
    } else if error.kind() == ErrorKind::Timeout || known(&["timed out", "deadline exceeded"]) {
        "the repository did not answer in time"
    } else if status("401") || known(&["unauthorized", "authentication required"]) {
        "the repository refused the credentials (401); check the user name and password"
    } else if status("403") || known(&["forbidden"]) {
        "access to the repository is forbidden (403)"
    } else if status("404") {
        "nothing is served at that URL (404); is it the chart repository's address?"
    } else if known(&[
        "no such host",
        "connection refused",
        "dial tcp",
        "network is unreachable",
    ]) {
        "the repository could not be reached; check the URL and your network"
    } else if known(&["x509", "certificate signed", "certificate verify", "tls:"]) {
        "the repository's certificate was not accepted; provide its CA file"
    } else if known(&[
        "is not a valid chart repository",
        "looks like",
        "index.yaml",
    ]) {
        "the URL does not serve a valid chart repository index"
    } else if known(&["already exists"]) {
        "a repository with that name already exists"
    } else if known(&["no repo named", "no repositories"]) {
        "there is no repository with that name"
    } else if known(&[
        "is locked",
        "lock file",
        "file lock",
        "temporarily unavailable",
    ]) {
        "Helm's repository file is locked by another Helm process; try again"
    } else {
        return text;
    };
    format!("{reason}. {text}")
}
