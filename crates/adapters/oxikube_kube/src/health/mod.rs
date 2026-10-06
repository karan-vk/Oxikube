//! Connection health and RBAC capabilities for one cluster context.
//!
//! * `liveness`: a probe loop ([`Liveness`]) that reports [`HealthEvent`]s, with backoff
//!   and a failure policy (one failure is `Degraded`, repeated or non-retryable failures
//!   are `Error`). The session manager (E06-S01) maps events onto `ClusterSessionState`.
//! * `capabilities`: a pure reduction of RBAC rules to a [`CapabilityReport`] (granted,
//!   restricted to named objects, unknown, denied).
//! * `rules`: `SelfSubjectRulesReview` per (context, namespace) behind a TTL cache.
//! * `access`: [`can_i`], a single `SelfSubjectAccessReview`.
//! * `pooled`: [`probe_context`], [`capabilities_for_context`] and [`pooled_probe`], which
//!   run the probes through the `ClientPool` and rebuild the client once after a
//!   retryable auth failure.
//!
//! # Lifecycle and failure policy
//!
//! Start the liveness loop once the session is `Ready`; `Healthy` and `Unhealthy` are not
//! legal session events while it is still `Connecting`. The first failed probe reports
//! `Unhealthy` (Degraded). The run ends with `Failed` (Error) after `failure_threshold`
//! consecutive failures (default 3), or at once for a permanent failure: a non-retryable
//! `Auth` error or a server certificate the TLS handshake rejected (untrusted issuer,
//! expired, wrong name). Other non-retryable kinds, such as a 403 on
//! `/version` from a hardened cluster, count toward the threshold. After `Failed` the loop
//! stops: there is no `Error` -> `Healthy` transition, so recovery is a reconnect
//! (`Connect`) by the session manager, which then restarts the loop.
//!
//! # `MutationGuard` does not apply here
//!
//! `SelfSubjectRulesReview` and `SelfSubjectAccessReview` are `POST` requests, but they
//! create no cluster state: the apiserver evaluates the review and answers it without
//! storing anything, and Oxikube persists nothing about them. They are not mutations in
//! the sense of ADR 0012, so they bypass the guard and are allowed in read-only mode. The
//! health probe itself is a `GET`.
//!
//! # Error handling
//!
//! Every failure is classified with [`crate::auth::classify_with`]; messages are safe to
//! show. Absence is a visible state: an incomplete review yields *unknown* capabilities,
//! never silently "denied", and a failing probe emits an event rather than being swallowed.
//!
//! Everything here is `async` and runs on Tokio; nothing blocks the UI thread.

mod access;
mod capabilities;
mod liveness;
mod pooled;
mod rules;

#[cfg(test)]
mod tests;

pub use access::{AccessDecision, AccessQuery, can_i};
pub use capabilities::{
    AccessLevel, AccessRule, CapabilityReport, RBAC_DERIVED, RulesSnapshot, capabilities_from_rules,
};
pub use liveness::{HealthEvent, Liveness, LivenessConfig, MIN_INTERVAL, probe_apiserver_version};
pub use pooled::{capabilities_for_context, pooled_probe, probe_context, rules_for_context};
pub use rules::{DEFAULT_RULES_TTL, RulesCache, fetch_rules, probe_capabilities};
