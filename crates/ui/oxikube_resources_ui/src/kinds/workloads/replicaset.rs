//! The ReplicaSet kind's spec.

use super::{ACTIONS_WITH_HISTORY, panel};
use crate::kinds::KindSpec;
use crate::kinds::builtin::workloads::spec_at;

/// ReplicaSet: the Workloads section's entry, `workload::Scale`, `workload::Rollback` and the workload panel.
pub(super) fn spec() -> KindSpec {
    spec_at("ReplicaSet", ACTIONS_WITH_HISTORY).with_detail(panel::renderer())
}
