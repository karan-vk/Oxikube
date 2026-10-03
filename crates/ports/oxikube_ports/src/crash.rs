//! [`CrashReporterPort`]: local crash capture with opt-in submission.
//!
//! # Adapter
//!
//! Implemented by `oxikube_crash`, which writes redacted crash reports to the local
//! data directory and, only after the user agrees, submits them. Oxikube is
//! telemetry-free: nothing leaves the machine from this port without
//! [`submit`](CrashReporterPort::submit) being called for a specific report.
//!
//! Reports are redacted by the producer before they reach the port (no kubeconfig
//! content, tokens or Secret data in `summary` or `backtrace`).

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;

/// Identifies a stored crash report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CrashId(pub String);

/// A redacted crash report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// Identifier, unique per report.
    pub id: CrashId,
    /// When the crash happened.
    pub ts: Timestamp,
    /// Application version that crashed.
    pub app_version: String,
    /// One-line description (panic message, redacted).
    pub summary: String,
    /// Backtrace text, redacted.
    pub backtrace: String,
}

/// Stores crash reports and submits them on request.
///
/// # Effects
///
/// Mutates only the local crash directory ([`record`](Self::record),
/// [`discard`](Self::discard)) and, after explicit consent, sends one report
/// ([`submit`](Self::submit)). Not a cluster mutation.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) when `submit` names an unknown
/// [`CrashId`], [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) when submission fails,
/// [`Internal`](oxikube_domain::ErrorKind::Internal) for local I/O failures.
#[async_trait]
pub trait CrashReporterPort: Send + Sync {
    /// Stores `report` locally. Does not send it.
    async fn record(&self, report: &CrashReport) -> OxiResult<()>;

    /// Reports stored and not yet submitted or discarded, oldest first.
    async fn pending(&self) -> OxiResult<Vec<CrashReport>>;

    /// Sends the report to the maintainers. Called only after explicit user
    /// consent for this report.
    async fn submit(&self, id: &CrashId) -> OxiResult<()>;

    /// Deletes the stored report. Returns whether it existed.
    async fn discard(&self, id: &CrashId) -> OxiResult<bool>;
}
