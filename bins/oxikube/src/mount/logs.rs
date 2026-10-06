//! The log service of the main window (E08-S01): the app's one `LogService`, which every log
//! viewer opens its sessions on.
//!
//! [`install`] builds it on first use over the Tokio bridge and the app clock, keeps it on the
//! [`AppState`], and makes `logs.buffer_lines` follow the settings: editing it in `settings.json`
//! trims or grows the sessions that are open, at once. Building it spawns nothing: a session's
//! task starts when a viewer opens one.

use std::sync::Arc;

use gpui::App;
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_logs_ui::{LogsSettings, follow_settings, log_runtime};
use oxikube_ports::ClockPort;
use oxikube_settings::Settings as _;

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
