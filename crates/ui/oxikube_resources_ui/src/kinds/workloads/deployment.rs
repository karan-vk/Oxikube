//! The Deployment kind's spec.

use super::{ROLLING_ACTIONS, panel};
use crate::kinds::KindSpec;
use crate::kinds::builtin::workloads::spec_at;

/// Deployment: the Workloads section's entry, scale, restart and set image, and the workload panel.
pub(super) fn spec() -> KindSpec {
    spec_at("Deployment", ROLLING_ACTIONS).with_detail(panel::renderer())
}
