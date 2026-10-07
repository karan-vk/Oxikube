//! Bounded reads of a log for readers that are not a viewer (E08-S09): the agent's `get_logs`
//! tool and its `@logs` mention.
//!
//! [`LogService::read_excerpt`](super::LogService::read_excerpt) opens a session that does not
//! follow, waits for it to end, and returns the newest matching lines as one redacted text with
//! the notes a reader needs to trust it ([`LogExcerpt`]). Everything is bounded: lines
//! ([`MAX_TAIL`]), bytes ([`MAX_EXCERPT_BYTES`]), time ([`READ_DEADLINE`]) and, for a workload,
//! streams (`logs.max_streams`, the aggregation's own cap).
//!
//! | Piece | Where |
//! |---|---|
//! | what is read and the limits; the `since` grammar | [`ExcerptRequest`], [`ExcerptSource`], [`parse_since`] (`request`) |
//! | which lines, written as redacted text within the budget | `render` |
//! | the result and its notes | [`LogExcerpt`] (`result`) |
//! | which cluster's ports | [`LogClusters`], [`LogCluster`] (`clusters`) |
//! | the read itself | `read` |
//!
//! # Redaction
//!
//! The text passes through [`oxikube_domain::redact::redact`] before it leaves this module (the
//! viewer is unaffected: it shows the user's own lines as written). Redaction is a best-effort
//! backstop for free-text logs: it masks tokens, `Authorization` values, credentials in URLs,
//! JWTs, PEM keys and Secret-like fields, but a secret in an unrecognised shape passes through.

mod clusters;
#[cfg(test)]
pub(crate) mod harness;
mod read;
mod render;
mod request;
mod result;
#[cfg(test)]
mod tests;

pub use clusters::{LogCluster, LogClusters};
pub use request::{
    DEFAULT_TAIL, ExcerptRequest, ExcerptSource, MAX_EXCERPT_BYTES, MAX_SINCE, MAX_TAIL,
    READ_DEADLINE, SCAN_LINES, parse_since, workload_kind,
};
pub use result::LogExcerpt;
