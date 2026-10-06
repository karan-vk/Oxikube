//! `logs.buffer_lines` through the real settings store: the default in `default.json`, the schema,
//! and a hot reload into a `LogService` with an open session.

use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::logs::{
    DEFAULT_BUFFER_LINES, LogConfig, LogRuntime, LogService, LogTarget, MIN_BUFFER_LINES,
};
use oxikube_app::store::Spawner;
use oxikube_ports::LogOptions;
use oxikube_settings::{Settings as _, SettingsStore};
use oxikube_testkit::{FakeClockPort, FakeLogPort, Timeline};
use parking_lot::Mutex;

use crate::{LogsSettings, follow_settings};

fn store() -> SettingsStore {
    SettingsStore::new(oxikube_assets::default_settings()).expect("the embedded defaults")
}

fn service() -> Arc<LogService> {
    let queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>> = Arc::default();
    let spawner: Arc<dyn Spawner> = Arc::new(move |task| queue.lock().push(task));
    Arc::new(LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(FakeClockPort::default()),
        },
        LogConfig::default(),
    ))
}

#[gpui::test]
fn the_default_json_holds_the_services_default(cx: &mut TestAppContext) {
    cx.set_global(store());
    cx.update(|cx| {
        assert_eq!(
            LogsSettings::get_global(cx).buffer_lines,
            DEFAULT_BUFFER_LINES,
            "assets/settings/default.json and oxikube_app::logs::DEFAULT_BUFFER_LINES disagree"
        );
    });
}

#[gpui::test]
fn the_schema_describes_the_setting(cx: &mut TestAppContext) {
    cx.set_global(store());
    cx.update(|cx| {
        let schema = cx.global::<SettingsStore>().json_schema();
        let text = serde_json::to_string(&schema).unwrap();
        assert!(text.contains("buffer_lines"), "{text}");
    });
}

#[gpui::test]
fn editing_the_setting_reaches_the_service_and_its_open_sessions(cx: &mut TestAppContext) {
    cx.set_global(store());
    let service = service();
    let port = Arc::new(FakeLogPort::new());
    port.script()
        .stream_logs
        .push_ok(Timeline::new().keep_open());
    let session = service.open(
        port,
        LogTarget::pod("default", "web-0"),
        LogOptions::follow(),
    );

    cx.update(|cx| follow_settings(&service, cx));
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
    session.read(|buffer, _| assert_eq!(buffer.capacity(), DEFAULT_BUFFER_LINES));

    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 2000 } }"#)
                .expect("valid settings");
        });
    });
    assert_eq!(service.buffer_lines(), 2_000);
    session.read(|buffer, _| assert_eq!(buffer.capacity(), 2_000));

    // A silly value is clamped, not applied raw.
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 3 } }"#)
                .expect("valid settings");
        });
    });
    assert_eq!(service.buffer_lines(), MIN_BUFFER_LINES);

    // An unrelated edit does not touch it.
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "ui_scale": 1.25 }"#)
                .expect("valid settings");
        });
    });
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
}
