//! The ReplicaSet kind's spec.

use super::{ACTIONS, panel};
use crate::kinds::KindSpec;
use crate::kinds::builtin::workloads::spec_at;

/// ReplicaSet: the Workloads section's entry, `workload::Scale`, and the workload panel.
pub(super) fn spec() -> KindSpec {
    spec_at("ReplicaSet", ACTIONS).with_detail(panel::renderer())
}
