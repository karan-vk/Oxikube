//! Per-cluster settings reach the session manager (E06-S08).
//!
//! `oxikube_settings` resolves `clusters.<id>` and `oxikube_app` has no `gpui`, so this binary
//! carries the values across: [`follow_cluster_settings`] pushes the resolved
//! [`ClusterPrefsTable`](oxikube_ports::ClusterPrefsTable) into the
//! [`ClusterSessionManager`] now and after every change of the cluster settings (a hot reload of
//! `settings.json`, or an in-app edit such as the read-only toggle). The manager applies only
//! what changed, to the clusters it changed for, without reconnecting.
//!
//! The other direction (E06-S09): [`SettingsPrefsWriter`] is the `oxikube_app::PrefsWriter` the
//! posture commands (`cluster::ToggleReadOnly`, `cluster::SetColour`, `cluster::ApplyPreset`)
//! persist through; it applies each edit on the foreground with the comment-preserving settings
//! editor, and the edit flows back through [`follow_cluster_settings`] to the live session.

mod writer;

use gpui::{App, Subscription};
use oxikube_app::ClusterSessionManager;
use oxikube_settings::{ClusterSettings, Settings as _};

/// Pushes the per-cluster settings into `manager` now and whenever they change.
///
/// Call it once the manager exists (after the settings store). Keep the returned subscription,
/// or `.detach()` it to follow for the life of the app. The push is a hash swap plus one short
/// lock per open session: it is safe on the UI thread and never touches a connection.
pub fn follow_cluster_settings(cx: &mut App, manager: ClusterSessionManager) -> Subscription {
    push(cx, &manager);
    ClusterSettings::observe(cx, move |cx| push(cx, &manager))
}

fn push(cx: &App, manager: &ClusterSessionManager) {
    manager.set_prefs_table(ClusterSettings::table(cx));
}

pub use writer::SettingsPrefsWriter;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod writer_tests;
