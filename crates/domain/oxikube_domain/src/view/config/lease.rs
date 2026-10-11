//! [`LeaseSummary`]: who holds a Lease and how fresh it is.

use std::sync::Arc;

use jiff::Timestamp;

use super::super::{ViewError, arc_of, check_kind, i64_of, sub, ts_of};
use crate::resource::Resource;

/// A coordination Lease: a leader election lock or a node heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseSummary {
    /// `spec.holderIdentity`: empty when nobody holds it.
    pub holder: Option<Arc<str>>,
    /// `spec.leaseDurationSeconds`.
    pub duration_seconds: Option<i64>,
    /// `spec.acquireTime`.
    pub acquired: Option<Timestamp>,
    /// `spec.renewTime`.
    pub renewed: Option<Timestamp>,
    /// `spec.leaseTransitions`: how often the holder changed.
    pub transitions: Option<i64>,
    /// `spec.preferredHolder` (coordinated leader election).
    pub preferred_holder: Option<Arc<str>>,
}

impl LeaseSummary {
    /// The summary of a `Lease`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a Lease.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Lease", &[("coordination.k8s.io", "Lease")])?;
        let spec = sub(res.json(), "spec");
        Ok(Self {
            holder: arc_of(spec, "holderIdentity"),
            duration_seconds: i64_of(spec, "leaseDurationSeconds"),
            acquired: ts_of(spec, "acquireTime"),
            renewed: ts_of(spec, "renewTime"),
            transitions: i64_of(spec, "leaseTransitions"),
            preferred_holder: arc_of(spec, "preferredHolder"),
        })
    }

    /// The `Holder` cell: the holder's identity, empty when the lease is free.
    pub fn holder_text(&self) -> &str {
        self.holder.as_deref().unwrap_or("")
    }
}
