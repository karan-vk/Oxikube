//! Authentication handling and error classification for the kube adapter.
//!
//! # Auth methods
//!
//! kube-rs builds the credential from the kubeconfig user, in this precedence: an
//! `auth-provider` (OIDC, GCP via command or OAuth), basic auth, an inline `token`, a
//! `token-file` (re-read about once a minute), an `exec` credential plugin (token and
//! client-cert output, cached until `expirationTimestamp`, run on the blocking pool when
//! it refreshes), and finally the client certificate. The `oauth` and `oidc` kube
//! features are enabled workspace-wide. Nothing here shells out to `kubectl`.
//!
//! # Modules
//!
//! * [`classify`]: `kube::Error` -> [`OxiError`](oxikube_domain::OxiError) ([`classify()`], [`classify_with`]).
//! * `exec`: [`ExecInteractivePolicy`], the cap on exec-plugin interactivity, and
//!   [`build_client`], which applies it and builds a client (blocking; `ClientPool`
//!   runs it on the blocking pool under a deadline).
//! * `refresh`: [`RefreshGuardLayer`], the deadline on a credential refresh inside a live
//!   client (a hung exec plugin fails requests with [`RefreshStalled`] instead of queueing
//!   them).
//! * [`retry_once`]: invalidate-and-retry-once for auth failures.
//!
//! # The `retryable` rule for `Auth`
//!
//! `retryable` is true when rebuilding the client could help: a 401 on a credential that
//! has a refresh path (exec, auth-provider, token file; see [`CredentialRefresh`]), and an
//! exec plugin that failed in a way that may be transient. It is false when a person has
//! to act: a static credential the server rejects (revoked), a missing or non-executable
//! plugin, a plugin waiting for an MFA code or sign-in, or a malformed credential. After
//! [`retry_once`] has rebuilt and failed again the flag is cleared, so by the time a
//! health probe (E03-S05) sees an `Auth` error with `retryable = true` it has not yet been
//! retried; `retryable = false` means the session should go to `AuthRequired`.
//!
//! # Secrets
//!
//! Error text is built from fixed phrases plus free text redacted with
//! [`oxikube_domain::redact::redact`]. The exec command line (arguments and environment),
//! plugin stdout, request headers and credentials never appear in an error message or source.

mod classify;
mod exec;
mod refresh;
mod retry;

pub(crate) use classify::redacted_line;
pub use classify::{
    CredentialRefresh, classify, classify_kubeconfig, classify_tls_setup, classify_with,
};
pub(crate) use exec::describe;
pub use exec::{ExecInteractivePolicy, build_client, build_client_bounded};
pub use refresh::{DEFAULT_REFRESH_DEADLINE, RefreshGuard, RefreshGuardLayer, RefreshStalled};
pub use retry::{retry_once, retry_once_kube};
