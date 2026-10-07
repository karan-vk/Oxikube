//! `kube::Error` -> [`OxiError`] classification.
//!
//! Free functions, because the orphan rule forbids `From<kube::Error> for OxiError`
//! in this crate. Messages are user-readable and never carry credential material: no
//! exec-plugin command line, no plugin stdout, no request headers. Free text that is
//! included (an API `Status` message, plugin stderr) is redacted with
//! [`oxikube_domain::redact::redact`] and then shaped to one bounded line (`text`).

use std::error::Error as StdError;
use std::io;

use kube::config::AuthInfo;
use kube::core::Status;
use oxikube_domain::OxiError;
use oxikube_domain::redact::redact;

mod auth_error;
mod cert_error;
mod config_error;
mod text;
mod upgrade_error;
use super::refresh::RefreshStalled;
use auth_error::classify_auth;
use cert_error::certificate_rejection;
pub use config_error::{classify_kubeconfig, classify_tls_setup};
use text::one_line;
use upgrade_error::classify_upgrade;

/// Whether the credential in use can be renewed by rebuilding the client.
///
/// This decides the `retryable` flag of a 401: a refresh helps an exec plugin, an OIDC
/// or GCP provider token or a rotated token file, and does nothing for a static token,
/// basic auth or a client certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredentialRefresh {
    /// Exec plugin, auth provider or token file: rebuilding re-runs the refresh.
    Refreshable,
    /// Inline token, basic auth or client certificate: a rebuild yields the same credential.
    Static,
    /// The caller does not know. A 401 is treated as retryable once; [`retry_once`](super::retry_once)
    /// clears the flag if the retry fails too.
    #[default]
    Unknown,
}

impl CredentialRefresh {
    /// Derives the answer from a kubeconfig user, mirroring kube's credential precedence.
    pub fn of(auth: &AuthInfo) -> Self {
        if auth.auth_provider.is_some() {
            return Self::Refreshable;
        }
        if (auth.username.is_some() && auth.password.is_some()) || auth.token.is_some() {
            return Self::Static;
        }
        if auth.token_file.is_some() || auth.exec.is_some() {
            return Self::Refreshable;
        }
        Self::Static
    }

    fn allows_retry(self) -> bool {
        self != Self::Static
    }
}

/// Free server text made safe for an error message: redacted, one bounded line, long opaque
/// runs masked. For adapters that build their own error detail (the write path's field causes).
pub(crate) fn redacted_line(text: &str) -> String {
    one_line(&redact(text))
}

/// Classifies a kube error. Equivalent to [`classify_with`] with [`CredentialRefresh::Unknown`].
///
/// | kube error | [`OxiError`] |
/// |---|---|
/// | 401, exec / OIDC / OAuth / token-file failures | `Auth` (see [`classify_with`] for `retryable`) |
/// | 403 | `Forbidden`, with the server's "who cannot do what" message |
/// | 404 / 409 (and 410 Gone) / 400, 422 | `NotFound` / `Conflict` / `Validation` |
/// | websocket upgrade (exec, attach, port-forward) refused with 401 / 403 / 404 | `Auth` / `Forbidden` / `NotFound` |
/// | 429, 503, 504, other upgrade failures, connection and TLS failures | `Network` |
/// | server certificate rejected in the handshake | `Network`, not retryable |
/// | 408, elapsed deadlines | `Timeout` |
/// | missing API group, 405/406/415/501 | `Unsupported` |
/// | everything else | `Internal` |
pub fn classify(err: &kube::Error) -> OxiError {
    classify_with(err, CredentialRefresh::Unknown)
}

/// Classifies a kube error knowing whether the credential can be refreshed.
///
/// `Auth.retryable` is true when rebuilding the client could help: an expired token with
/// a refresh path, or an exec plugin that failed in a way that may be transient. It is
/// false for a credential that is missing, malformed, revoked (a 401 on a
/// [`Static`](CredentialRefresh::Static) credential) or that needs a human (a plugin
/// asking for an MFA code, a missing plugin binary).
pub fn classify_with(err: &kube::Error, refresh: CredentialRefresh) -> OxiError {
    use kube::Error as E;
    match err {
        E::Api(status) => classify_status(status, refresh),
        E::Auth(auth) => classify_auth(auth),
        E::Service(inner) => classify_chain(inner.as_ref(), refresh),
        E::HyperError(e) if e.is_timeout() => {
            OxiError::timeout("the request to the cluster timed out")
        }
        E::HyperError(e) => transport("connection to the cluster failed", e),
        E::ReadEvents(e) => io_error("reading the event stream failed", e),
        E::RustlsTls(e) => classify_tls_setup(e),
        E::TlsRequired => OxiError::internal("TLS is required but no TLS stack is available"),
        E::ProxyProtocolUnsupported { .. } | E::ProxyProtocolDisabled { .. } => {
            OxiError::unsupported("the configured proxy protocol is not supported")
        }
        E::Discovery(d) => classify_discovery(d),
        E::InferKubeconfig(e) => classify_kubeconfig(e),
        // In-cluster inference is not used by the adapter; keep kube's text out anyway,
        // since it embeds the kubeconfig error too.
        E::InferConfig(_) => OxiError::internal("could not infer a client configuration"),
        E::UpgradeConnection(e) => classify_upgrade(e, refresh),
        E::SerdeError(_) | E::FromUtf8(_) | E::LinesCodecMaxLineLengthExceeded => {
            OxiError::internal("the cluster sent a response that could not be decoded")
        }
        E::HttpError(_) | E::BuildRequest(_) => OxiError::internal("could not build the request"),
        #[allow(unreachable_patterns)]
        _ => OxiError::internal(format!(
            "unexpected client error: {}",
            one_line(&redact(&err.to_string()))
        )),
    }
}

fn classify_status(status: &Status, refresh: CredentialRefresh) -> OxiError {
    let msg = one_line(&redact(&status.message));
    let code = status.code;
    let reason = status.reason.as_str();
    // Match the code as well as the reason: `Status::is_*` ignore the code when the reason is
    // empty, and kube synthesises code-only statuses for bodies that are not a `Status`.
    if code == 401 || reason == "Unauthorized" {
        let detail = if msg.is_empty() {
            String::new()
        } else {
            format!(": {msg}")
        };
        let hint = if refresh.allows_retry() {
            "the cluster rejected the credentials (they may have expired)"
        } else {
            "the cluster rejected the credentials; sign in again or update the kubeconfig"
        };
        return OxiError::auth(format!("{hint}{detail}"), refresh.allows_retry());
    }
    if code == 403 || reason == "Forbidden" {
        // The apiserver text already reads "pods is forbidden: User \"u\" cannot list
        // resource \"pods\" in API group \"\" in the namespace \"x\"" (identity, verb,
        // resource); it never contains the credential.
        let msg = if msg.is_empty() {
            "access denied".to_owned()
        } else {
            msg
        };
        return OxiError::forbidden(msg);
    }
    if code == 404 || reason == "NotFound" {
        return OxiError::not_found(or_default(msg, "not found"));
    }
    if matches!(code, 409 | 410)
        || matches!(reason, "Conflict" | "AlreadyExists" | "Gone" | "Expired")
    {
        return OxiError::conflict(or_default(msg, "conflict with the current state"));
    }
    if matches!(code, 400 | 413 | 422)
        || matches!(reason, "Invalid" | "BadRequest" | "RequestEntityTooLarge")
    {
        return OxiError::validation(or_default(msg, "invalid request"));
    }
    if code == 408 || matches!(reason, "Timeout" | "ServerTimeout") {
        return OxiError::timeout(or_default(msg, "the cluster timed out"));
    }
    if matches!(code, 429 | 503 | 504) || matches!(reason, "TooManyRequests" | "ServiceUnavailable")
    {
        return OxiError::network(format!(
            "the cluster is unavailable ({}): {msg}",
            code_or_reason(status)
        ));
    }
    if matches!(code, 405 | 406 | 415 | 501)
        || matches!(
            reason,
            "MethodNotAllowed" | "NotAcceptable" | "UnsupportedMediaType"
        )
    {
        return OxiError::unsupported(or_default(msg, "the cluster does not support this request"));
    }
    OxiError::internal(format!(
        "unexpected API error ({}): {msg}",
        code_or_reason(status)
    ))
}

fn code_or_reason(status: &Status) -> String {
    if status.code != 0 {
        status.code.to_string()
    } else if status.reason.is_empty() {
        "unknown".to_owned()
    } else {
        status.reason.clone()
    }
}

fn or_default(msg: String, default: &str) -> String {
    if msg.is_empty() {
        default.to_owned()
    } else {
        msg
    }
}

fn classify_discovery(err: &kube::error::DiscoveryError) -> OxiError {
    use kube::error::DiscoveryError as D;
    match err {
        D::InvalidGroupVersion(_) => OxiError::validation("invalid group/version"),
        D::MissingKind(_) | D::MissingApiGroup(_) | D::MissingResource(_) | D::EmptyApiGroup(_) => {
            OxiError::unsupported(one_line(&redact(&err.to_string())))
        }
    }
}

/// Walks a boxed transport error chain: kube wraps middleware (auth layer, buffer,
/// timeout) failures in `Error::Service`, so an auth failure or an `io::Error` may sit
/// several `source()` hops down.
fn classify_chain(err: &(dyn StdError + 'static), refresh: CredentialRefresh) -> OxiError {
    let mut cur: Option<&(dyn StdError + 'static)> = Some(err);
    while let Some(e) = cur {
        if let Some(stalled) = e.downcast_ref::<RefreshStalled>() {
            return OxiError::auth(
                format!(
                    "{stalled}; the exec credential plugin may be hung, so the connection is rebuilt on retry"
                ),
                true,
            );
        }
        if let Some(auth) = e.downcast_ref::<kube::client::AuthError>() {
            return classify_auth(auth);
        }
        if let Some(kube_err) = e.downcast_ref::<kube::Error>() {
            return classify_with(kube_err, refresh);
        }
        if let Some(io_err) = e.downcast_ref::<io::Error>() {
            return io_error("connection to the cluster failed", io_err);
        }
        cur = e.source();
    }
    transport("request to the cluster failed", err)
}

fn io_error(context: &str, err: &io::Error) -> OxiError {
    if err.kind() == io::ErrorKind::TimedOut {
        return OxiError::timeout(format!("{context}: timed out"));
    }
    transport(context, err)
}

/// A transport failure: a non-retryable `Network` error when the handshake rejected the
/// server certificate, `Timeout` when the text says a deadline elapsed, else `Network`.
fn transport(context: &str, err: &(dyn StdError + 'static)) -> OxiError {
    if let Some(rejected) = certificate_rejection(err) {
        return rejected;
    }
    let text = one_line(&redact(&err.to_string()));
    let lower = text.to_ascii_lowercase();
    if lower.contains("timed out")
        || lower.contains("deadline has elapsed")
        || lower.contains("timeout")
    {
        OxiError::timeout(format!("{context}: timed out")).with_source(text)
    } else {
        OxiError::network(format!("{context}: {text}")).with_source(text)
    }
}
