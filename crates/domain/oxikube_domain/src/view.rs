//! View-models: typed projections of core kinds, built from [`Resource`] JSON.
//!
//! Tables, detail headers and agent summaries all need the same answers ("what status is this
//! pod", "is this deployment ready", "is this node schedulable"). The answers live here once:
//!
//! * [`PodSummary`] and [`ContainerSummary`] for Pods, with the `STATUS` column computed by the
//!   same rules as `kubectl get pods`.
//! * [`NodeSummary`] for Nodes.
//! * [`WorkloadSummary`] for Deployments, StatefulSets, DaemonSets and ReplicaSets.
//! * [`JobSummary`] and [`CronJobSummary`] for batch kinds.
//! * [`Health`] and [`health_of`]: the one rule for "is this object healthy" behind overview
//!   tiles and sidebar counts.
//!
//! Every constructor reads the raw JSON (ADR 0005: never `k8s-openapi`). Missing or wrongly
//! typed fields degrade to defaults instead of failing, because old and new API servers both
//! leave gaps. The only error is passing the wrong kind ([`ViewError::WrongKind`]).
//!
//! # Provenance
//!
//! The pod, node, job and workload column rules follow the semantics of the Kubernetes printers
//! (`pkg/printers/internalversion/printers.go`, Apache-2.0). They are reimplemented here; no code
//! is copied. kdash's `get_status` is deliberately not used (its init-container branch is wrong).
//!
//! # Performance
//!
//! Constructors run once per watch delta per row (10 000 pods with 1 % churn every 5 s is the
//! reference load). They walk the JSON by key without cloning subtrees and allocate only for the
//! output strings; name and namespace are `Arc` clones of [`ObjectMeta`](crate::ObjectMeta).

use std::sync::Arc;

use crate::json::{Array, JsonRef};
use crate::resource::Resource;
use jiff::Timestamp;

mod health;
mod job;
mod node;
mod pod;
mod pod_health;
mod workload;

pub use health::{Health, has_health_rule, health_of};
pub use job::{CronJobSummary, JobStatus, JobSummary};
pub use node::NodeSummary;
pub use pod::{
    ContainerKind, ContainerState, ContainerSummary, PodPhase, PodSummary, QosClass,
    TerminatedState,
};
pub use workload::{WorkloadKind, WorkloadSummary};

/// Why a view-model could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// The resource is not of a kind this view-model accepts.
    #[error("expected {expected}, got {found}")]
    WrongKind {
        /// The kind (or kinds) the constructor accepts, for example `Pod`.
        expected: &'static str,
        /// The resource's group-qualified kind, for example `apps/Deployment`.
        found: String,
    },
}

/// Status of a Kubernetes condition (`True`, `False`, `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConditionStatus {
    /// The condition holds.
    True,
    /// The condition does not hold.
    False,
    /// The controller cannot tell, or the field is missing or unrecognised.
    #[default]
    Unknown,
}

impl ConditionStatus {
    /// Parse the `status` string of a condition; anything unrecognised is `Unknown`.
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("True") => Self::True,
            Some("False") => Self::False,
            _ => Self::Unknown,
        }
    }

    /// The API spelling: `True`, `False` or `Unknown`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::True => "True",
            Self::False => "False",
            Self::Unknown => "Unknown",
        }
    }
}

/// One entry of a `status.conditions` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// The condition `type`, for example `Ready` or `MemoryPressure`.
    pub kind: Arc<str>,
    /// Whether the condition holds.
    pub status: ConditionStatus,
    /// Machine-readable `reason`, when set.
    pub reason: Option<Arc<str>>,
    /// Human-readable `message`, when set.
    pub message: Option<Arc<str>>,
    /// `lastTransitionTime`, when set and parseable.
    pub last_transition: Option<Timestamp>,
}

impl Condition {
    /// Parse one condition object. Returns `None` when it has no `type`.
    fn from_json(v: JsonRef<'_>) -> Option<Self> {
        Some(Self {
            kind: Arc::from(str_of(v, "type")?),
            status: ConditionStatus::parse(str_of(v, "status")),
            reason: arc_of(v, "reason"),
            message: arc_of(v, "message"),
            last_transition: ts_of(v, "lastTransitionTime"),
        })
    }

    /// Whether `status` is `True`.
    pub fn is_true(&self) -> bool {
        self.status == ConditionStatus::True
    }
}

// --- shared JSON readers ------------------------------------------------------
//
// All of them accept any value and return a default for non-objects, so a wrongly typed
// subtree (for example `"status": "oops"`) degrades instead of panicking. They borrow from the
// document and allocate nothing.

/// Reject `res` unless its group and kind match one of `accepted`.
fn check_kind(
    res: &Resource,
    expected: &'static str,
    accepted: &[(&str, &str)],
) -> Result<(), ViewError> {
    let (group, kind) = (&*res.kind.group, &*res.kind.kind);
    if accepted.iter().any(|&(g, k)| g == group && k == kind) {
        Ok(())
    } else if group.is_empty() {
        Err(ViewError::WrongKind {
            expected,
            found: kind.to_owned(),
        })
    } else {
        Err(ViewError::WrongKind {
            expected,
            found: format!("{group}/{kind}"),
        })
    }
}

/// `v[key]`, or `null` when absent or `v` is not an object.
fn sub<'a>(v: JsonRef<'a>, key: &str) -> JsonRef<'a> {
    v.get(key).unwrap_or(JsonRef::NULL)
}

/// `v[key]` when it is a JSON object.
fn obj_of<'a>(v: JsonRef<'a>, key: &str) -> Option<JsonRef<'a>> {
    v.get(key).filter(|x| x.is_object())
}

/// `v[key]` as a non-empty string.
fn str_of<'a>(v: JsonRef<'a>, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// `v[key]` as a non-empty shared string.
fn arc_of(v: JsonRef<'_>, key: &str) -> Option<Arc<str>> {
    str_of(v, key).map(Arc::from)
}

/// `v[key]` as an array; empty when absent or not an array.
fn arr_of<'a>(v: JsonRef<'a>, key: &str) -> Array<'a> {
    v.get(key)
        .and_then(JsonRef::as_array)
        .unwrap_or(Array::EMPTY)
}

/// `v[key]` as an integer.
fn i64_of(v: JsonRef<'_>, key: &str) -> Option<i64> {
    v.get(key)?.as_i64()
}

/// `v[key]` as a boolean; `false` when absent or not a boolean.
fn bool_of(v: JsonRef<'_>, key: &str) -> bool {
    v.get(key).and_then(JsonRef::as_bool).unwrap_or(false)
}

/// `v[key]` as a non-negative count; `None` when absent, negative or not an integer.
fn opt_count(v: JsonRef<'_>, key: &str) -> Option<u32> {
    let n = v.get(key)?.as_u64()?;
    Some(u32::try_from(n).unwrap_or(u32::MAX))
}

/// `v[key]` as a non-negative count; zero when absent, negative or not an integer.
fn count_of(v: JsonRef<'_>, key: &str) -> u32 {
    opt_count(v, key).unwrap_or(0)
}

/// `v[key]` as an `i32` (exit codes, signals); zero when absent or out of range.
fn i32_of(v: JsonRef<'_>, key: &str) -> i32 {
    i64_of(v, key)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

/// `v[key]` as an RFC 3339 timestamp; `None` when absent or unparseable.
fn ts_of(v: JsonRef<'_>, key: &str) -> Option<Timestamp> {
    str_of(v, key)?.parse().ok()
}
