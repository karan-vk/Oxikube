//! Classification of client auth errors (exec plugin, OIDC, OAuth, token file).

use std::io;

use kube::client::AuthError;
use oxikube_domain::OxiError;

use crate::auth::scrub::{scrub, scrub_tail};

/// Classifies a client auth error (exec plugin, OIDC, OAuth, token file, ...).
pub(super) fn classify_auth(err: &AuthError) -> OxiError {
    use kube::client::AuthError as A;
    match err {
        A::InvalidBasicAuth(_) | A::InvalidBearerToken(_) => OxiError::auth(
            "the configured credential is not a valid HTTP header value",
            false,
        ),
        A::UnrefreshableTokenResponse => OxiError::auth(
            "the credential plugin returned a credential that cannot be refreshed",
            false,
        ),
        A::ExecPluginFailed => {
            OxiError::auth("the exec credential plugin returned no credential", true)
        }
        A::MalformedTokenExpirationDate(_) => OxiError::auth(
            "the exec credential plugin returned an invalid expirationTimestamp",
            false,
        ),
        A::AuthExecStart(e) => match e.kind() {
            io::ErrorKind::NotFound => OxiError::auth(
                "the exec credential plugin was not found; install it or fix `command` in the kubeconfig",
                false,
            ),
            io::ErrorKind::PermissionDenied => {
                OxiError::auth("the exec credential plugin is not executable", false)
            }
            _ => OxiError::auth("the exec credential plugin could not be started", true),
        },
        A::AuthExecRun { status, out, .. } => exec_run_failed(&status.to_string(), &out.stderr),
        A::AuthExecParse(_) => OxiError::auth(
            "the exec credential plugin printed output that is not a valid ExecCredential",
            false,
        ),
        A::AuthExecSerialize(_) => {
            OxiError::internal("could not encode the exec credential request")
        }
        A::ExecMissingClusterInfo | A::MissingCommand => OxiError::auth(
            "the exec credential plugin configuration is incomplete",
            false,
        ),
        A::AuthExec(msg) => {
            OxiError::auth(format!("credential helper failed: {}", scrub(msg)), true)
        }
        A::ReadTokenFile(e, path) => {
            let permanent = matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            );
            OxiError::auth(
                format!("cannot read the token file {}", path.display()),
                !permanent,
            )
        }
        A::ParseTokenKey(_) => {
            OxiError::auth("the credential provider's token-key is invalid", false)
        }
        A::OAuth(e) => {
            use kube::client::OAuthError as O;
            // Fetching a token is a network round trip, so transport-ish failures may clear up.
            let transient = matches!(
                e,
                O::RequestToken(_)
                    | O::RetrieveCredentials(_)
                    | O::ConcatBuffers(_)
                    | O::ParseToken(_)
                    | O::Unknown(_)
            );
            OxiError::auth("could not obtain an OAuth token for the cluster", transient)
        }
        A::Oidc(e) => classify_oidc(e),
        A::NoValidNativeRootCA(_) => {
            OxiError::internal("no valid native root CA certificates were found")
        }
        #[allow(unreachable_patterns)]
        _ => OxiError::auth("authentication failed", false),
    }
}

fn classify_oidc(err: &kube::client::oidc_errors::Error) -> OxiError {
    use kube::client::oidc_errors::{Error as O, RefreshError as R};
    match err {
        O::Refresh(R::HyperError(_) | R::HyperUtilError(_)) => OxiError::auth(
            "refreshing the OIDC token failed (provider unreachable)",
            true,
        ),
        O::Refresh(R::RequestFailed(status)) if status.is_server_error() => {
            OxiError::auth("refreshing the OIDC token failed (provider error)", true)
        }
        O::Refresh(_) => {
            OxiError::auth("the OIDC refresh token was rejected; sign in again", false)
        }
        O::IdTokenMissing | O::IdToken(_) | O::RefreshInit(_) => OxiError::auth(
            "the OIDC ID token has expired and cannot be refreshed; sign in again",
            false,
        ),
    }
}

/// Failure of an exec plugin that ran and exited non-zero. `stderr` is the plugin's own
/// message; stdout (which may hold a credential) is never read.
fn exec_run_failed(status: &str, stderr: &[u8]) -> OxiError {
    let stderr = String::from_utf8_lossy(stderr);
    let needs_input = needs_human_input(&stderr);
    let tail = scrub_tail(&stderr, 3);
    let detail = if tail.is_empty() {
        String::new()
    } else {
        format!(": {tail}")
    };
    if needs_input {
        OxiError::auth(
            format!(
                "the exec credential plugin needs you to sign in or answer a prompt, which Oxikube cannot do ({status}){detail}. Run the plugin's sign-in in a terminal, then reconnect"
            ),
            false,
        )
    } else {
        OxiError::auth(
            format!("the exec credential plugin failed ({status}){detail}"),
            true,
        )
    }
}

/// Whole-word / whole-phrase markers (matched on alphanumeric words, so `login.microsoftonline.com`,
/// `pretty` and `footprint` do not count) that a plugin is waiting for a person.
const INPUT_WORDS: &[&str] = &["mfa", "2fa", "otp", "tty", "stdin", "interactive"];
const INPUT_PHRASES: &[&str] = &[
    "one time password",
    "one time code",
    "device code",
    "verification code",
    "enter code",
    "enter the code",
    "enter your code",
    "press enter",
    "not a terminal",
    "no terminal",
    "for prompt",
    "cannot prompt",
    "unable to prompt",
    "please log in",
    "please login",
    "please sign in",
    "login required",
    "sign in required",
    "log in again",
    "sign in again",
    "az login",
    "sso login",
    "auth login",
];

/// True when plugin stderr says it needs a person (MFA code, device-code sign-in, a tty).
fn needs_human_input(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.iter().any(|w| INPUT_WORDS.contains(w)) {
        return true;
    }
    let padded = format!(" {} ", words.join(" "));
    INPUT_PHRASES
        .iter()
        .any(|p| padded.contains(&format!(" {p} ")))
}

#[cfg(test)]
mod tests {
    use super::needs_human_input;

    #[test]
    fn prompts_and_sign_in_requests_need_a_person() {
        for s in [
            "Enter MFA code for arn:aws:iam::0:mfa/dummy:",
            "error: no terminal available for prompt",
            "stdin is not a terminal",
            "Please log in: az login",
            "To sign in, use a web browser and enter the device code ABCD",
            "error: SSO session expired, run aws sso login",
            "open /dev/tty: no such device (no tty)",
        ] {
            assert!(needs_human_input(s), "{s}");
        }
    }

    #[test]
    fn network_failures_and_lookalike_words_do_not() {
        for s in [
            "Post https://login.microsoftonline.com/tenant/oauth2/token: dial tcp: i/o timeout",
            "a pretty kitty with a footprint",
            "error: credentials service unavailable",
            "failed to get token: terminal error from server",
        ] {
            assert!(!needs_human_input(s), "{s}");
        }
    }
}
