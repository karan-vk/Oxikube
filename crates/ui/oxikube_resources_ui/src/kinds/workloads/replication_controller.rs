//! The ReplicationController kind's spec.

use super::{ACTIONS, panel};
use crate::kinds::KindSpec;
use crate::kinds::builtin::workloads::spec_at;

/// ReplicationController: the Workloads section's entry, `workload::Scale`, and the workload panel.
pub(super) fn spec() -> KindSpec {
    spec_at("ReplicationController", ACTIONS).with_detail(panel::renderer())
}
