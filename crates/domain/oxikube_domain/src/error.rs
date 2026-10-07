//! Error taxonomy: the single error type every port returns.
//!
//! [`OxiError`] carries an [`ErrorKind`] that the UI, the session state machine
//! and agents react to (re-authenticate on [`ErrorKind::Auth`], show "no
//! permission" on [`ErrorKind::Forbidden`], retry when `retryable`), a message
//! that is safe to show, an optional source error and an explicit `retryable`
//! flag.
//!
//! # Rules
//!
//! * The domain has no `From<kube::Error>`; adapters expose a free function or
//!   an extension trait instead (the orphan rule forbids `From` there). See the
//!   mapping table in `docs/ARCHITECTURE.md`.
//! * This type never redacts. Adapters must strip tokens and Secret data
//!   *before* building an error, e.g. with [`redact::redact`](crate::redact::redact).
//! * Errors are not `Clone` (the boxed source is not). Wrap in `Arc` where
//!   sharing is needed.
//!
//! The Rust representation is a struct with a `kind`, not an enum of variants.
//! Read the kind with [`OxiError::kind`] and match on [`ErrorKind`]; build
//! errors with the constructors ([`OxiError::auth`], [`OxiError::not_found`], ...).

use std::error::Error as StdError;
use std::fmt;

/// Convenient result alias used by ports and services.
pub type OxiResult<T> = Result<T, OxiError>;

/// Boxed source error attached to an [`OxiError`].
type BoxedSource = Box<dyn StdError + Send + Sync + 'static>;

/// Classification of an [`OxiError`]. Callers branch on this, never on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Credentials missing, expired or rejected (HTTP 401).
    Auth,
    /// Authenticated but not permitted (HTTP 403).
    Forbidden,
    /// The resource or API object does not exist (HTTP 404).
    NotFound,
    /// The request conflicts with current state (HTTP 409).
    Conflict,
    /// Transport-level or transient server failure (connection reset, 429, 503, 504).
    Network,
    /// A client or request deadline elapsed.
    Timeout,
    /// The request was malformed or rejected by validation (HTTP 400, 422).
    Validation,
    /// The server or cluster does not support the feature (missing API group, ...).
    Unsupported,
    /// A bug or an unexpected condition (including panics turned into errors).
    Internal,
    /// A local resource limit refused the request: the per-cluster watch budget (feed or
    /// object cap) would be exceeded. The message says which limit and how to free room
    /// (close views, narrow the namespace selection). Not retryable until something is
    /// released.
    BudgetExceeded,
}

impl ErrorKind {
    /// Every kind, in declaration order.
    pub const ALL: [ErrorKind; 10] = [
        ErrorKind::Auth,
        ErrorKind::Forbidden,
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Network,
        ErrorKind::Timeout,
        ErrorKind::Validation,
        ErrorKind::Unsupported,
        ErrorKind::Internal,
        ErrorKind::BudgetExceeded,
    ];

    /// Stable variant name, e.g. `"NotFound"`. Used in docs, logs and tool output.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Auth => "Auth",
            ErrorKind::Forbidden => "Forbidden",
            ErrorKind::NotFound => "NotFound",
            ErrorKind::Conflict => "Conflict",
            ErrorKind::Network => "Network",
            ErrorKind::Timeout => "Timeout",
            ErrorKind::Validation => "Validation",
            ErrorKind::Unsupported => "Unsupported",
            ErrorKind::Internal => "Internal",
            ErrorKind::BudgetExceeded => "BudgetExceeded",
        }
    }

    /// Default `retryable` value for the kind: transient kinds ([`Network`](Self::Network),
    /// [`Timeout`](Self::Timeout)) are retryable, everything else is not. Override per
    /// error with [`OxiError::with_retryable`].
    pub const fn default_retryable(self) -> bool {
        matches!(self, ErrorKind::Network | ErrorKind::Timeout)
    }

    /// The kind whose [`Display`](fmt::Display) label is `label` (`"internal error"` gives
    /// [`ErrorKind::Internal`]): reads back the kind of a rendered [`OxiError`], for text that
    /// crossed a boundary as a plain string (a session's failure reason).
    pub fn from_label(label: &str) -> Option<ErrorKind> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }

    /// Short human label for UI use, e.g. `"not found"`.
    const fn label(self) -> &'static str {
        match self {
            ErrorKind::Auth => "authentication failed",
            ErrorKind::Forbidden => "forbidden",
            ErrorKind::NotFound => "not found",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Network => "network error",
            ErrorKind::Timeout => "timed out",
            ErrorKind::Validation => "invalid request",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Internal => "internal error",
            ErrorKind::BudgetExceeded => "budget exceeded",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The error returned by every port.
///
/// `Display` is `"<kind label>: <message>"` and is safe to show in the UI provided
/// the message was redacted by the adapter. The source is reachable through
/// [`std::error::Error::source`] and is deliberately not part of `Display`.
#[derive(thiserror::Error)]
#[error("{kind}: {message}")]
pub struct OxiError {
    kind: ErrorKind,
    message: String,
    #[source]
    source: Option<BoxedSource>,
    retryable: bool,
}

impl OxiError {
    /// Builds an error of `kind` with the kind's default `retryable` value.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
            retryable: kind.default_retryable(),
        }
    }

    /// [`ErrorKind::Auth`]. `retryable` is explicit: an exec-plugin refresh can be
    /// retried, a revoked token cannot.
    pub fn auth(message: impl Into<String>, retryable: bool) -> Self {
        Self::new(ErrorKind::Auth, message).with_retryable(retryable)
    }

    /// [`ErrorKind::Forbidden`] (not retryable).
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Forbidden, message)
    }

    /// [`ErrorKind::NotFound`] (not retryable).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// [`ErrorKind::Conflict`] (not retryable; the caller must re-read first).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, message)
    }

    /// [`ErrorKind::Network`], retryable by default. Use `.with_retryable(false)`
    /// for a failure known to be permanent.
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, message)
    }

    /// [`ErrorKind::Timeout`], retryable by default.
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Timeout, message)
    }

    /// [`ErrorKind::Validation`] (not retryable).
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Validation, message)
    }

    /// [`ErrorKind::Unsupported`] (not retryable).
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// [`ErrorKind::Internal`] (not retryable).
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    /// [`ErrorKind::BudgetExceeded`] (not retryable): `message` is the human-readable reason.
    pub fn budget_exceeded(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::BudgetExceeded, message)
    }

    /// Attaches the underlying error. The caller is responsible for it carrying no
    /// secrets (redact before wrapping).
    #[must_use]
    pub fn with_source(mut self, source: impl Into<BoxedSource>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Overrides the `retryable` flag.
    #[must_use]
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// The error's classification.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The message, without the kind prefix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether repeating the operation may succeed.
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Debug for OxiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OxiError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("retryable", &self.retryable)
            .field("source", &self.source.as_ref().map(|s| s.to_string()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const _: fn() = || {
        fn assert_traits<T: Send + Sync + 'static>() {}
        assert_traits::<OxiError>();
        assert_traits::<ErrorKind>();
    };

    #[derive(Debug)]
    struct Leaf(&'static str);
    impl fmt::Display for Leaf {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }
    impl StdError for Leaf {}

    #[test]
    fn each_constructor_sets_its_kind_and_default_retryable() {
        let cases: [(OxiError, ErrorKind, bool); 10] = [
            (OxiError::auth("a", false), ErrorKind::Auth, false),
            (OxiError::forbidden("a"), ErrorKind::Forbidden, false),
            (OxiError::not_found("a"), ErrorKind::NotFound, false),
            (OxiError::conflict("a"), ErrorKind::Conflict, false),
            (OxiError::network("a"), ErrorKind::Network, true),
            (OxiError::timeout("a"), ErrorKind::Timeout, true),
            (OxiError::validation("a"), ErrorKind::Validation, false),
            (OxiError::unsupported("a"), ErrorKind::Unsupported, false),
            (OxiError::internal("a"), ErrorKind::Internal, false),
            (
                OxiError::budget_exceeded("a"),
                ErrorKind::BudgetExceeded,
                false,
            ),
        ];
        for (err, kind, retryable) in cases {
            assert_eq!(err.kind(), kind);
            assert_eq!(err.is_retryable(), retryable, "{kind:?}");
            assert_eq!(err.message(), "a");
        }
    }

    #[test]
    fn all_lists_every_kind_once() {
        let mut names: Vec<_> = ErrorKind::ALL.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn auth_retryable_is_explicit() {
        assert!(OxiError::auth("exec plugin refresh", true).is_retryable());
        assert!(!OxiError::auth("token revoked", false).is_retryable());
    }

    #[test]
    fn retryable_can_be_overridden_both_ways() {
        assert!(
            !OxiError::network("gone")
                .with_retryable(false)
                .is_retryable()
        );
        assert!(
            OxiError::conflict("stale")
                .with_retryable(true)
                .is_retryable()
        );
    }

    #[test]
    fn display_is_kind_then_message() {
        assert_eq!(
            OxiError::not_found("pod web-1").to_string(),
            "not found: pod web-1"
        );
        assert_eq!(ErrorKind::Auth.to_string(), "authentication failed");
    }

    #[test]
    fn type_does_not_alter_message_with_token_shaped_text() {
        // Redaction is the adapter's job; the type must pass the text through untouched.
        let msg = "request failed: Authorization: Bearer FAKE.TOKEN.123";
        let err = OxiError::internal(msg);
        assert_eq!(err.message(), msg);
        assert!(err.to_string().ends_with(msg));
        assert!(format!("{err:?}").contains("FAKE.TOKEN.123"));
    }

    #[test]
    fn source_chain_prints_through_error_source() {
        let err = OxiError::network("watch failed")
            .with_source(Box::new(Leaf("connection reset")) as Box<dyn StdError + Send + Sync>);
        let src = StdError::source(&err).expect("source");
        assert_eq!(src.to_string(), "connection reset");
        assert!(StdError::source(src).is_none());
        // Display excludes the source; Debug includes its text.
        assert_eq!(err.to_string(), "network error: watch failed");
        assert!(format!("{err:?}").contains("connection reset"));
    }

    #[test]
    fn with_source_accepts_a_string() {
        let err = OxiError::internal("x").with_source("inner");
        assert_eq!(StdError::source(&err).unwrap().to_string(), "inner");
    }

    #[test]
    fn nested_oxierror_chains() {
        let inner = OxiError::timeout("deadline");
        let outer = OxiError::network("list failed").with_source(inner);
        let first = StdError::source(&outer).unwrap();
        assert_eq!(first.to_string(), "timed out: deadline");
    }

    #[test]
    fn no_source_by_default() {
        assert!(StdError::source(&OxiError::forbidden("x")).is_none());
    }

    #[test]
    fn architecture_doc_mapping_table_lists_every_kind() {
        let doc = include_str!("../../../../docs/ARCHITECTURE.md");
        for kind in ErrorKind::ALL {
            assert!(
                doc.contains(&format!("| `{}` |", kind.as_str())),
                "docs/ARCHITECTURE.md error mapping table is missing {kind:?}"
            );
        }
    }

    fn any_kind() -> impl Strategy<Value = ErrorKind> {
        prop::sample::select(ErrorKind::ALL.to_vec())
    }

    proptest! {
        #[test]
        fn new_preserves_kind_message_and_default(kind in any_kind(), msg in ".*") {
            let err = OxiError::new(kind, msg.clone());
            prop_assert_eq!(err.kind(), kind);
            prop_assert_eq!(err.message(), msg.as_str());
            prop_assert_eq!(err.is_retryable(), kind.default_retryable());
            prop_assert_eq!(err.to_string(), format!("{kind}: {msg}"));
        }

        #[test]
        fn with_retryable_always_wins(kind in any_kind(), flag: bool) {
            prop_assert_eq!(OxiError::new(kind, "m").with_retryable(flag).is_retryable(), flag);
        }
    }
}
