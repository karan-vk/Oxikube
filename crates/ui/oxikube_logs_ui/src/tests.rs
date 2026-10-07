//! `logs.buffer_lines` through the real settings store: the default in `default.json` and a hot
//! reload, global and per cluster, into a `LogService` with open sessions.

use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::logs::{
    DEFAULT_BUFFER_LINES, LogConfig, LogRuntime, LogService, LogTarget, MIN_BUFFER_LINES,
};
use oxikube_app::store::Spawner;
use oxikube_domain::ids::{ClusterId, ContextName};
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
    cx.run_until_parked();
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
    session.read(|buffer, _| assert_eq!(buffer.capacity(), DEFAULT_BUFFER_LINES));

    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "logs": { "buffer_lines": 2000 } }"#)
                .expect("valid settings");
        });
    });
    cx.run_until_parked();
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
    cx.run_until_parked();
    assert_eq!(service.buffer_lines(), MIN_BUFFER_LINES);

    // An unrelated edit does not touch it.
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "ui_scale": 1.25 }"#)
                .expect("valid settings");
        });
    });
    cx.run_until_parked();
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);
}

fn set_user(cx: &mut TestAppContext, text: &str) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(text).expect("valid settings");
        });
    });
    cx.run_until_parked();
}

fn open_in(
    service: &LogService,
    cluster: &ClusterId,
) -> (Arc<FakeLogPort>, oxikube_app::logs::LogSession) {
    let port = Arc::new(FakeLogPort::new());
    port.script()
        .stream_logs
        .push_ok(Timeline::new().keep_open());
    let session = service.open_in(
        cluster,
        port.clone(),
        LogTarget::pod("default", "web-0"),
        LogOptions::follow(),
    );
    (port, session)
}

#[gpui::test]
fn a_cluster_override_resizes_that_clusters_sessions_only(cx: &mut TestAppContext) {
    let prod = ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"));
    let dev = ClusterId::new("/home/me/.kube/config", &ContextName::new("dev"));
    cx.set_global(store());
    let service = service();
    let (_p, on_prod) = open_in(&service, &prod);
    let (_d, on_dev) = open_in(&service, &dev);
    cx.update(|cx| follow_settings(&service, cx));
    cx.run_until_parked();
    on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), DEFAULT_BUFFER_LINES));

    set_user(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{}": {{ "logs": {{ "buffer_lines": 2000 }} }} }} }}"#,
            prod.as_str()
        ),
    );
    on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 2_000));
    on_dev.read(|buffer, _| assert_eq!(buffer.capacity(), DEFAULT_BUFFER_LINES));
    assert_eq!(service.buffer_lines_for(&prod), 2_000);
    assert_eq!(service.buffer_lines(), DEFAULT_BUFFER_LINES);

    // The user's own value moves everything that has no override of its own.
    set_user(
        cx,
        &format!(
            r#"{{ "logs": {{ "buffer_lines": 5000 }},
                  "clusters": {{ "{}": {{ "logs": {{ "buffer_lines": 2000 }} }} }} }}"#,
            prod.as_str()
        ),
    );
    on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 2_000));
    on_dev.read(|buffer, _| assert_eq!(buffer.capacity(), 5_000));

    // Removing the override puts the cluster back on the default.
    set_user(cx, r#"{ "logs": { "buffer_lines": 5000 } }"#);
    on_prod.read(|buffer, _| assert_eq!(buffer.capacity(), 5_000));
}
