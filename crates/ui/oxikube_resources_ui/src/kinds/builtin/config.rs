//! Config: ConfigMaps and Secrets have modules of their own (`kinds::configmap`, `kinds::secret`,
//! E12-S11; ranks 10 and 20). Quotas and limits, autoscalers, disruption budgets, priority and
//! runtime classes, leases, admission webhooks and policies are `kinds::config` (E12-S12), which
//! registers them after these two (ranks 30 and up).

use gpui::App;

use super::super::{configmap, secret};

/// Registers the Config section's kinds.
pub fn init(cx: &mut App) {
    configmap::init(cx);
    secret::init(cx);
    crate::kinds::config::init(cx);
}
