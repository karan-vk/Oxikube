//! Mapping HTTP statuses, Prometheus `errorType`s and transport failures onto [`PromqlError`].
//!
//! Every message that reaches an error passes through [`redact`], and transport errors are
//! stripped of their URL first (it carries the query string).

use std::error::Error as _;
use std::time::Duration;

use oxikube_domain::redact::redact;
use oxikube_ports::PromqlError;

/// The longest server message kept in an error. A misbehaving endpoint could answer with a page
/// of HTML; the UI shows one line.
const MAX_MESSAGE_CHARS: usize = 300;

/// A server-provided message made safe to show: redacted, single-line, bounded.
pub(crate) fn clean(message: &str) -> String {
    let redacted = redact(message);
    let mut line = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > MAX_MESSAGE_CHARS {
        line = line.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + "...";
    }
    line
}

/// A failure the server reported: either a non-2xx status, or `status: "error"` in the envelope.
/// `error_type` and `message` come from the envelope when the body had one.
pub(crate) fn from_server(status: u16, error_type: &str, message: &str) -> PromqlError {
    let text = |fallback: &str| {
        let cleaned = clean(message);
        if cleaned.is_empty() {
            fallback.to_owned()
        } else {
            cleaned
        }
    };
    match status {
        401 => return PromqlError::Unauthorized(text("401 Unauthorized")),
        403 => return PromqlError::Forbidden(text("403 Forbidden")),
        404 => {
            return PromqlError::BadResponse(format!(
                "404 Not Found: {} (check the endpoint's path prefix)",
                text("no Prometheus API at this URL")
            ));
        }
        _ => {}
    }
    // The envelope's errorType is the precise signal; VictoriaMetrics answers with the status
    // code as the type ("422").
    match error_type {
        "bad_data" | "execution" | "422" | "400" => {
            return PromqlError::BadQuery {
                message: text("the server rejected the query"),
            };
        }
        "timeout" => {
            return PromqlError::ServerError {
                status,
                message: format!("the query timed out on the server: {}", text("timeout")),
            };
        }
        _ => {}
    }
    match status {
        400 | 413 | 414 | 422 => PromqlError::BadQuery {
            message: text("the server rejected the query"),
        },
        429 | 500..=599 => PromqlError::ServerError {
            status,
            message: text(&format!("HTTP {status}")),
        },
        _ => PromqlError::BadResponse(format!(
            "HTTP {status}: {}",
            text("unexpected status for a query")
        )),
    }
}

/// A transport-level failure of the request or of reading its body.
pub(crate) fn from_transport(error: reqwest::Error, timeout: Duration) -> PromqlError {
    // The URL (and so the query) must not leak into the message.
    let error = error.without_url();
    if error.is_timeout() {
        return PromqlError::Timeout(timeout);
    }
    let chain = chain_text(&error);
    if error.is_builder() {
        return PromqlError::InvalidEndpoint(clean(&chain));
    }
    if looks_like_tls(&chain) {
        return PromqlError::TlsError(clean(&chain));
    }
    if error.is_decode() {
        return PromqlError::BadResponse(clean(&chain));
    }
    PromqlError::Unreachable(clean(&chain))
}

/// The whole `source()` chain as one string, outermost first.
fn chain_text(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(next) = source {
        let text = next.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
        source = next.source();
    }
    parts.join(": ")
}

/// rustls surfaces certificate and handshake failures as I/O errors wrapping its `Error`; the
/// types are not reachable through reqwest, so the chain text is matched instead.
fn looks_like_tls(chain: &str) -> bool {
    let lower = chain.to_ascii_lowercase();
    [
        "certificate",
        "tls",
        "handshake",
        "invalid peer",
        "unknownissuer",
        "notvalidforname",
        "corrupt message",
        "invalidcontenttype",
        "peer incompatible",
        "alert",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_the_taxonomy() {
        assert!(matches!(
            from_server(401, "", ""),
            PromqlError::Unauthorized(_)
        ));
        assert!(
            matches!(from_server(403, "", "denied"), PromqlError::Forbidden(m) if m == "denied")
        );
        assert!(
            matches!(from_server(404, "", ""), PromqlError::BadResponse(m) if m.contains("path prefix"))
        );
        assert!(matches!(
            from_server(400, "bad_data", "parse error"),
            PromqlError::BadQuery { .. }
        ));
        assert!(matches!(
            from_server(422, "execution", "dup"),
            PromqlError::BadQuery { .. }
        ));
        assert!(matches!(
            from_server(422, "422", "vm"),
            PromqlError::BadQuery { .. }
        ));
        assert!(matches!(
            from_server(400, "", "plain"),
            PromqlError::BadQuery { .. }
        ));
        assert!(matches!(
            from_server(500, "internal", ""),
            PromqlError::ServerError { status: 500, .. }
        ));
        assert!(matches!(
            from_server(503, "unavailable", ""),
            PromqlError::ServerError { status: 503, .. }
        ));
        assert!(
            matches!(from_server(503, "timeout", "deadline"), PromqlError::ServerError { status: 503, message } if message.contains("timed out"))
        );
        assert!(matches!(
            from_server(429, "", ""),
            PromqlError::ServerError { status: 429, .. }
        ));
        assert!(matches!(
            from_server(302, "", ""),
            PromqlError::BadResponse(_)
        ));
    }

    #[test]
    fn messages_are_redacted_single_line_and_bounded() {
        let msg = clean("line one\n  Authorization: Bearer abc.def.ghi\nline two");
        assert!(!msg.contains("abc.def.ghi"), "{msg}");
        assert!(!msg.contains('\n'));
        let long = clean(&"x".repeat(5000));
        assert!(long.chars().count() <= MAX_MESSAGE_CHARS + 3);
    }

    #[test]
    fn tls_failures_are_recognised_from_the_chain() {
        assert!(looks_like_tls(
            "error trying to connect: invalid peer certificate: UnknownIssuer"
        ));
        assert!(looks_like_tls(
            "received corrupt message of type InvalidContentType"
        ));
        assert!(!looks_like_tls(
            "tcp connect error: Connection refused (os error 61)"
        ));
    }
}
