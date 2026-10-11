//! [`ScalePlan`] and [`ScaleResult`], and the checks on the number a user types.

use std::sync::Arc;

use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::WorkloadKind;
use oxikube_domain::{OxiError, OxiResult};

/// The most replicas a scale may ask for: what the API's `int32` holds.
pub const MAX_REPLICAS: u32 = i32::MAX as u32;

/// Whether a kind has a `scale` subresource a user sets: Deployment, StatefulSet, ReplicaSet and
/// ReplicationController. A DaemonSet does not.
pub fn is_scalable(gvk: &Gvk) -> bool {
    WorkloadKind::of(gvk).is_some_and(WorkloadKind::scalable)
}

/// Reads the number a scale dialog's field holds.
///
/// # Errors
///
/// `Validation`, in words for the field: empty, not a whole number, negative, or beyond
/// [`MAX_REPLICAS`]. Surrounding blanks are ignored.
pub fn parse_replicas(text: &str) -> OxiResult<u32> {
    let text = text.trim();
    if text.is_empty() {
        return Err(OxiError::validation("Enter the number of replicas."));
    }
    if let Some(rest) = text.strip_prefix('-')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(OxiError::validation("Replicas cannot be negative."));
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(OxiError::validation(
            "Replicas must be a whole number, 0 or more.",
        ));
    }
    match text.parse::<u32>() {
        Ok(n) if n <= MAX_REPLICAS => Ok(n),
        _ => Err(OxiError::validation(format!(
            "Replicas cannot be more than {MAX_REPLICAS}."
        ))),
    }
}

/// What a scale dialog starts with: the workload as the cluster has it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalePlan {
    /// The workload.
    pub target: ResourceRef,
    /// Which controller it is.
    pub kind: WorkloadKind,
    /// Replicas that exist (`status.replicas` of the `scale` subresource).
    pub current: u32,
    /// Replicas wanted now (`spec.replicas`): what the dialog's field starts with.
    pub desired: u32,
    /// The name of a HorizontalPodAutoscaler that targets this workload, when one was found: it
    /// may set the count back. A soft hint: `None` when there is none or the lookup failed.
    pub autoscaler: Option<Arc<str>>,
}

/// What a scale did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleResult {
    /// The workload.
    pub target: ResourceRef,
    /// `spec.replicas` before, when the `scale` subresource could be read first.
    pub before: Option<u32>,
    /// `spec.replicas` after (what the server answered; for a dry run, what it would be).
    pub after: u32,
    /// The dispatch was a dry run: the server rehearsed the patch and nothing changed.
    pub dry_run: bool,
}

impl ScaleResult {
    /// The line to show: `Scaled Deployment shop/web from 3 to 5 replicas`.
    pub fn message(&self) -> String {
        let object = match self.target.namespace() {
            Some(ns) => format!("{ns}/{}", self.target.name),
            None => self.target.name.to_string(),
        };
        let kind = &self.target.gvk.kind;
        let noun = if self.after == 1 {
            "replica"
        } else {
            "replicas"
        };
        let from = self
            .before
            .map_or_else(String::new, |before| format!(" from {before}"));
        let verb = if self.dry_run {
            "Dry run: would scale"
        } else {
            "Scaled"
        };
        format!("{verb} {kind} {object}{from} to {} {noun}", self.after)
    }
}
