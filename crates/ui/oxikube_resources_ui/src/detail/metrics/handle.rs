//! How a detail view finds the app's history service: a GPUI global the binary installs once.

use gpui::{App, Global};
use oxikube_app::MetricsHistoryService;

/// The app's [`MetricsHistoryService`], installed by the binary when it has a PromQL port. A
/// detail view without it (a test, a build without metrics) has no Metrics tab.
#[derive(Clone)]
pub struct MetricsHistory(MetricsHistoryService);

impl Global for MetricsHistory {}

impl MetricsHistory {
    /// Makes `service` the one every detail view uses.
    pub fn install(service: MetricsHistoryService, cx: &mut App) {
        cx.set_global(Self(service));
    }

    /// The installed service, if any.
    pub fn get(cx: &App) -> Option<MetricsHistoryService> {
        cx.try_global::<Self>().map(|global| global.0.clone())
    }
}
