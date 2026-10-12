//! Node drain (`kubectl drain`) as data: the options, the plan, the progress events and the
//! board the drain dialog shows (E04-S07, E12-S10).
//!
//! The algorithm that cordons and evicts runs in the kube adapter over the `DrainPort`
//! (`oxikube_kube::KubeDrain`); its orchestration, guard and audit are `oxikube_app::nodes`'s.
//! Everything both sides and the UI share lives here, pure:
//!
//! | File | Holds |
//! |---|---|
//! | `options` | [`DrainOptions`] (the algorithm's knobs), [`PodRef`], [`DrainPlan`] with [`SkippedPod`] / [`BlockedPod`], [`DrainProgress`] (the stream's items), [`DrainSummary`] |
//! | `plan` | [`plan_drain`]: kubectl's filters, which pods are evicted, skipped or block the drain |
//! | `flags` | [`DrainFlags`]: the person's choices as `kubectl drain` flags (the `node::Drain` payload), their kubectl command line and audit words |
//! | `board` | [`DrainBoard`]: the progress events folded into one row per pod ([`PodDrainState`]: Pending, Evicting, Blocked by PDB, Terminating, Deleted, Failed) |

mod board;
mod flags;
mod options;
mod plan;

pub use board::{DrainBoard, DrainCounts, DrainPhase, DrainRow, PodDrainState};
pub use flags::{DEFAULT_TIMEOUT_SECS, DrainFlags, MAX_GRACE_PERIOD_SECS, MAX_TIMEOUT_SECS};
pub use options::{
    BlockReason, BlockedPod, DrainOptions, DrainPlan, DrainProgress, DrainSummary, PodRef,
    SkipReason, SkippedPod,
};
pub use plan::plan_drain;
