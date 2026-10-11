//! Config: ConfigMaps and Secrets here; quotas and limits, autoscalers, disruption budgets,
//! priority and runtime classes, leases, admission webhooks and policies are `kinds::config`
//! (E12-S12), which registers them after these two (ranks 30 and up).

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, generic, namespaced, register};

const CONFIG: &[Builtin] = &[
    namespaced("", "v1", "ConfigMap", "configmaps", "ConfigMaps"),
    namespaced("", "v1", "Secret", "secrets", "Secrets"),
];

/// Registers the Config section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::CONFIG, CONFIG, generic);
    crate::kinds::config::init(cx);
}
