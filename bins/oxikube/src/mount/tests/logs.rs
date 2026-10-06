//! The log service of the main window (E08-S01): built by the mount, shared by every window, and
//! bounded by `logs.buffer_lines` through the real settings store.

use std::sync::Arc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::logs::{DEFAULT_BUFFER_LINES, LogTarget};
use oxikube_ports::LogOptions;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{FakeLogPort, TestPorts, Timeline};

use super::App;
use crate::app_state::AppState;

#[gpui::test]
fn the_mount_builds_the_log_service_with_the_default_bound(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).log_service().cloned())
        .expect("the mount built the app's log service");
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
}

#[gpui::test]
fn a_user_setting_bounds_the_service_and_hot_reloads_into_open_sessions(cx: &mut TestAppContext) {
    let mut app = App::start_with(cx, TestPorts::seeded(), |cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 1500 } }"#)
                .expect("valid settings");
        });
    });
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).log_service().cloned())
        .expect("the log service");
    assert_eq!(service.buffer_lines(), 1_500, "built from the user's value");

    // An open session follows an edit made while it streams.
    let port = Arc::new(FakeLogPort::new());
    port.script()
        .stream_logs
        .push_ok(Timeline::new().keep_open());
    let session = service.open(
        port,
        LogTarget::pod("default", "web-0"),
        LogOptions::follow(),
    );
    app.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 400 } }"#)
                .expect("valid settings");
        });
    });
    assert_eq!(service.buffer_lines(), 400);
    session.read(|buffer, _| assert_eq!(buffer.capacity(), 400));
}
