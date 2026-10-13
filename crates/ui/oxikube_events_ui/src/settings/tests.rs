//! The `events` settings through the real store (default.json, the user's file, `clusters.<id>`)
//! and into a running `EventService`.

use std::sync::Arc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::ClusterSessionManager;
use oxikube_app::events::{
    DEFAULT_EVENTS_BUFFER, EventService, EventsConfig, MAX_EVENTS_BUFFER, MIN_EVENTS_BUFFER,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_settings::{Settings as _, SettingsStore};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};

use super::*;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    store.set_user_settings(user).expect("valid settings");
    store
}

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

fn buffer(value: Option<i64>) -> usize {
    EventsSettings::from_content(EventsContent { buffer: value }).buffer
}

#[test]
fn default_json_spells_out_the_default() {
    assert_eq!(
        store("{}").get::<EventsSettings>(None).buffer,
        DEFAULT_EVENTS_BUFFER,
        "assets/settings/default.json and oxikube_app's DEFAULT_EVENTS_BUFFER disagree"
    );
    let defaults = oxikube_settings::jsonc::parse_jsonc_object(oxikube_assets::default_settings())
        .expect("default.json parses");
    assert_eq!(defaults["events"]["buffer"], 10_000);
}

#[test]
fn out_of_range_values_are_clamped_not_refused() {
    assert_eq!(buffer(None), DEFAULT_EVENTS_BUFFER);
    assert_eq!(buffer(Some(2_500)), 2_500);
    assert_eq!(buffer(Some(3)), MIN_EVENTS_BUFFER);
    assert_eq!(buffer(Some(-1)), MIN_EVENTS_BUFFER);
    assert_eq!(buffer(Some(i64::MAX)), MAX_EVENTS_BUFFER);
    let store = store(r#"{ "events": { "buffer": 1 } }"#);
    assert_eq!(store.get::<EventsSettings>(None).buffer, MIN_EVENTS_BUFFER);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn a_cluster_can_have_its_own_bound() {
    let (prod, dev) = (cluster("prod"), cluster("dev"));
    let user = serde_json::json!({
        "events": { "buffer": 5000 },
        "clusters": { prod.to_string(): { "events": { "buffer": 20000 } } },
    });
    let store = store(&user.to_string());
    let config = EventsSettings::config(&store, EventsConfig::default());
    assert_eq!(config.buffer, 5_000);
    assert_eq!(config.buffer_for(&prod), 20_000);
    assert_eq!(config.buffer_for(&dev), 5_000);
}

fn service() -> EventService {
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(FakeClockPort::default()),
    );
    EventService::new(
        sessions,
        Arc::new(FakeClockPort::default()),
        EventsConfig::default(),
    )
}

#[gpui::test]
fn the_service_follows_edits_of_the_setting(cx: &mut TestAppContext) {
    cx.set_global(store("{}"));
    let service = service();
    let _follow = cx.update(|cx| follow_settings(&service, cx));
    cx.run_until_parked();
    assert_eq!(service.config().buffer, DEFAULT_EVENTS_BUFFER);

    let set = |cx: &mut TestAppContext, text: &str| {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, _| {
                store.set_user_settings(text).expect("valid settings");
            });
        });
        cx.run_until_parked();
    };
    set(cx, r#"{ "events": { "buffer": 2000 } }"#);
    assert_eq!(service.config().buffer, 2_000);
    set(cx, r#"{ "events": { "buffer": 3 } }"#);
    assert_eq!(service.config().buffer, MIN_EVENTS_BUFFER);
    let prod = cluster("prod");
    let user =
        serde_json::json!({ "clusters": { prod.to_string(): { "events": { "buffer": 300 } } } });
    set(cx, &user.to_string());
    let config = service.config();
    assert_eq!(config.buffer, DEFAULT_EVENTS_BUFFER);
    assert_eq!(config.buffer_for(&prod), 300);
    assert_eq!(
        config.idle_grace,
        EventsConfig::default().idle_grace,
        "timings are not settings"
    );
}
