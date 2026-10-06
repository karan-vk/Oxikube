//! [`follow_settings`]: `logs.buffer_lines` into the running `LogService`.

use std::sync::Arc;

use gpui::App;
use oxikube_app::logs::LogService;
use oxikube_settings::Settings as _;

use crate::LogsSettings;

/// Sets the service's bound from the current settings and again on every change of them: sessions
/// that are open are trimmed or given room at once, new ones start with the new bound. Without a
/// settings store the service keeps the bound it was built with.
pub fn follow_settings(service: &Arc<LogService>, cx: &mut App) {
    apply(service, cx);
    let service = service.clone();
    LogsSettings::observe(cx, move |cx| apply(&service, cx)).detach();
}

fn apply(service: &LogService, cx: &App) {
    if let Some(settings) = LogsSettings::try_get(cx) {
        service.set_buffer_lines(settings.buffer_lines);
    }
}
