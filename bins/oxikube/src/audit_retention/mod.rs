//! `audit.retention_days` and the daily prune of the audit log (E19-S08).
//!
//! [`start`] runs with the feature `init`s. It builds the [`AuditRetention`] scheduler over the
//! state database, starts it on the background executor (it prunes shortly after the first
//! frame's work is done, then once a day) and follows the `audit` setting, so an edit of
//! `retention_days` applies and prunes at once. The delete and the vacuum run on the state
//! store's own thread; nothing here touches the UI thread beyond a settings read.
//!
//! The scheduler is a GPUI task owned by [`AuditRetentionWiring`] (dropped with the app), not a
//! Tokio task: the idle app holds none.

use std::sync::Arc;

use gpui::{App, AppContext as _, Global, Subscription, Task};
use oxikube_app::AuditRetention;
use oxikube_settings::{AuditSettings, Settings as _};

use crate::app_state::AppState;

/// What keeps the scheduler and the settings follower alive for the life of the app.
pub struct AuditRetentionWiring {
    retention: Arc<AuditRetention>,
    _task: Task<()>,
    _subscription: Subscription,
}

impl Global for AuditRetentionWiring {}

impl AuditRetentionWiring {
    /// The scheduler, for tests and status displays.
    pub fn retention(&self) -> &Arc<AuditRetention> {
        &self.retention
    }
}

/// The configured window, or the default without a settings store.
fn configured_days(cx: &App) -> u32 {
    AuditSettings::try_get(cx).map_or(oxikube_settings::audit::DEFAULT_RETENTION_DAYS, |s| {
        s.retention_days
    })
}

/// Starts pruning the audit log and follows `audit.retention_days`. Does nothing without an
/// [`AppState`], and only once per app.
pub fn start(cx: &mut App) {
    let Some(state) = AppState::try_global(cx) else {
        return;
    };
    if cx.has_global::<AuditRetentionWiring>() {
        return;
    }
    let retention = Arc::new(AuditRetention::new(
        state.state().clone(),
        state.ports().clusters.clock.clone(),
        configured_days(cx),
    ));
    let task = cx.background_spawn({
        let retention = retention.clone();
        async move { retention.run().await }
    });
    let subscription = AuditSettings::observe(cx, {
        let retention = retention.clone();
        move |cx| retention.set_days(configured_days(cx))
    });
    cx.set_global(AuditRetentionWiring {
        retention,
        _task: task,
        _subscription: subscription,
    });
}

#[cfg(test)]
mod tests;
