//! `drain`: empty a Node of its pods the way `kubectl drain` does, as a progress stream.
//!
//! ```text
//! get node -> list pods on it -> plan (skip DaemonSet and mirror pods, refuse blockers)
//!   -> cordon -> list and plan again (pods that landed meanwhile) -> evict each pod (concurrently, a few at a time)
//!        429 from a PodDisruptionBudget -> back off and retry until the timeout
//!        accepted -> wait until the pod is gone
//!   -> Finished
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | [`DrainOptions`], [`DrainProgress`], [`DrainSummary`] and the pod types, [`plan_drain`] (which pods are evicted, skipped or block the drain) | `oxikube_domain::drain` (pure, shared with the app and the UI; E12-S10) |
//! | one pod: evict, retry on a budget, wait for it to go | `evict` |
//! | [`drain`] / [`drain_with`]: the stream; [`drain_to_completion`]: await it | `run` |
//! | [`KubeDrain`]: the `DrainPort` the app drains through (E12-S10) | `port` |
//!
//! # Differences from kubectl
//!
//! * The plan is made, and a drain a pod blocks is refused, **before** the node is cordoned;
//!   kubectl cordons first and then fails, leaving the node cordoned. Because pods can land
//!   in the gap, the node is listed and planned again right after the cordon and that second
//!   plan is the one carried out (and reported in the summary); a pod that only the second
//!   plan blocks fails the drain with the node cordoned, as in kubectl.
//! * `dry_run` makes no request that changes anything (not even a server dry run): it reports
//!   the plan and finishes, which is what "what would be evicted" needs.
//! * Pods are evicted a few at a time (`concurrency`), not one by one.
//! * A pod being evicted is identified by its uid: the eviction carries a uid precondition, so a
//!   replacement with the same name (a StatefulSet's) is never evicted by mistake, and "gone"
//!   means the object is missing or has another uid.
//!
//! Dropping the stream stops the drain: nothing runs in the background. The node stays cordoned.

mod evict;
mod port;
mod run;

pub use oxikube_domain::drain::{
    BlockReason, BlockedPod, DrainOptions, DrainPlan, DrainProgress, DrainSummary, PodRef,
    SkipReason, SkippedPod, plan_drain,
};
pub use port::KubeDrain;
pub use run::{drain, drain_to_completion, drain_with};
