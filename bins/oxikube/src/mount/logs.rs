//! The logs of the main window: the app's one `LogService` (E08-S01), which every log viewer
//! opens its sessions on, and the window's log views (E08-S02).
//!
//! [`install`] builds the service on first use over the Tokio bridge and the app clock, keeps it
//! on the [`AppState`], and makes `logs.buffer_lines` follow the settings: editing it in
//! `settings.json` trims or grows the sessions that are open, at once. Building it spawns nothing:
//! a session's task starts when a viewer opens one.
//!
//! [`start_views`] starts the window's [`LogViews`]: `pod::ViewLogs` (the "View Logs" row action
//! of a pod, the palette, an agent's `k8s.pod_view_logs`) opens a log view as a tab of the pod's
//! cluster tab, and the `logs::*` commands drive it. [`row_actions`] is the tables' row action
//! registry with "View Logs" in it.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::RowActionRegistry;
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_logs_ui::{
    LogRequest, LogViewDeps, LogViews, LogViewsDeps, LogsSettings, follow_settings,
    log_row_actions, log_runtime,
};
use oxikube_ports::{ClockPort, FsPort};
use oxikube_settings::Settings as _;
use oxikube_workspace::{ClusterTabs, CommandDispatcher};

use crate::app_state::AppState;

/// The app's `LogService`: the one already set on `state`, else a new one that is set (and follows
/// the settings).
pub fn install(state: &AppState, clock: Arc<dyn ClockPort>, cx: &mut App) -> Arc<LogService> {
    if let Some(service) = state.log_service() {
        return service.clone();
    }
    let config = LogConfig {
        buffer_lines: LogsSettings::try_get(cx)
            .map_or(LogConfig::default().buffer_lines, |s| s.buffer_lines),
        ..LogConfig::default()
    };
    let service = Arc::new(LogService::new(log_runtime(clock, cx), config));
    if !state.set_log_service(service.clone()) {
        // Another window set one first: use that, so every window shares the bound.
        return state.log_service().cloned().unwrap_or(service);
    }
    follow_settings(&service, cx);
    service
}

/// Starts the window's log views over the app's `service` (saving through `fs`): requests from `requests` (the bus's
/// log handlers) open views in the cluster tabs of `tabs`.
pub fn start_views(
    service: Arc<LogService>,
    sessions: ClusterSessionManager,
    fs: Arc<dyn FsPort>,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<LogRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<LogViews> {
    let deps = LogViewsDeps {
        views: LogViewDeps {
            service,
            sessions,
            dispatcher,
            fs,
        },
        host: Rc::new(tabs),
    };
    LogViews::start(deps, requests, window, cx)
}

/// The resource tables' row actions: the core ones (delete), the CRD list's (E07-S07) and "View
/// Logs" on pods (E08-S02).
pub fn row_actions() -> RowActionRegistry {
    let mut registry = RowActionRegistry::core();
    let specs = oxikube_resources_ui::crds::crd_row_actions()
        .into_iter()
        .chain(log_row_actions());
    for spec in specs {
        if let Err(error) = registry.register(spec) {
            // A wiring bug (one command offered twice); the tests catch it.
            tracing::error!(%error, "a row action was registered twice");
        }
    }
    registry
}
