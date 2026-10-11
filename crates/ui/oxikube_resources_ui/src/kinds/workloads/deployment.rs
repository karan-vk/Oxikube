//! The Deployment kind's spec.

use super::{ACTIONS, panel};
use crate::kinds::KindSpec;
use crate::kinds::builtin::workloads::spec_at;

/// Deployment: the Workloads section's entry, `workload::Scale`, and the workload panel.
pub(super) fn spec() -> KindSpec {
    spec_at("Deployment", ACTIONS).with_detail(panel::renderer())
}
