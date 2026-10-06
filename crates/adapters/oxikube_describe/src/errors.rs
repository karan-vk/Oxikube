//! Turning the failures of deskribe (plain strings) and of `kubectl` (stderr) into the error
//! taxonomy of `docs/ARCHITECTURE.md`. Every message is redacted first.

use oxikube_domain::OxiError;
use oxikube_domain::redact::redact;

/// The error for a failure described by `message`: `NotFound`, `Forbidden`, `Auth`, `Timeout` and
/// `Network` are recognised by what the API server and the clients say; anything else is
/// `Internal`. The message is redacted and trimmed.
pub(crate) fn classify(message: &str) -> OxiError {
    let message = redact(message.trim()).into_owned();
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if has(&["notfound", "not found", "code: 404"]) {
        OxiError::not_found(message)
    } else if has(&["forbidden", "code: 403"]) {
        OxiError::forbidden(message)
    } else if has(&["unauthorized", "code: 401", "you must be logged in"]) {
        OxiError::auth(message, false)
    } else if has(&["timed out", "timeout", "deadline exceeded"]) {
        OxiError::timeout(message)
    } else if has(&[
        "connection refused",
        "unable to connect",
        "error trying to connect",
        "connection reset",
        "no route to host",
        "dns error",
        "tcp connect",
    ]) {
        OxiError::network(message)
    } else {
        OxiError::internal(message)
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ErrorKind;

    use super::*;

    #[test]
    fn classifies_what_the_clients_say() {
        let kind = |text: &str| classify(text).kind();
        assert_eq!(
            kind(
                r#"describe GET failed: ApiError: pods "x" not found: NotFound (ErrorResponse { code: 404 })"#
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind("Error from server (Forbidden): pods \"x\" is forbidden"),
            ErrorKind::Forbidden
        );
        assert_eq!(
            kind("error: You must be logged in to the server (Unauthorized)"),
            ErrorKind::Auth
        );
        assert_eq!(kind("request timed out"), ErrorKind::Timeout);
        assert_eq!(
            kind(
                "Unable to connect to the server: dial tcp 127.0.0.1:6443: connect: connection refused"
            ),
            ErrorKind::Network
        );
        assert_eq!(kind("something odd"), ErrorKind::Internal);
    }

    #[test]
    fn messages_are_redacted() {
        let error = classify("failed: Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(
            !error.message().contains("abcdefghijklmnopqrstuvwxyz"),
            "{error}"
        );
    }
}
