//! `AppState` accessors on the test constructor.

use futures::executor::block_on;
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_ports::StateKey;
use oxikube_settings::SettingsStore;
use serde_json::json;

use super::AppState;

#[gpui::test]
fn the_state_port_is_the_testkit_fake(cx: &mut TestAppContext) {
    let state = cx.update(|cx| AppState::test(cx));
    let key = StateKey::new("k").unwrap();
    // The fake is plain in-memory data: no thread, no real wait.
    let read = block_on(async {
        state.state().kv_set(&key, json!("v")).await.unwrap();
        state.state().kv_get(&key).await.unwrap()
    });
    assert_eq!(read, Some(json!("v")));
}

#[gpui::test]
fn accessors_read_the_current_globals_not_a_copy(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let state = AppState::test(cx);
        let before = state
            .settings(cx)
            .generation::<oxikube_logging::LogSettings>();
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "log": { "filter": "debug" } }"#)
                .unwrap();
        });
        // The same `AppState` sees the hot-reloaded store.
        let after = state
            .settings(cx)
            .generation::<oxikube_logging::LogSettings>();
        assert_ne!(before, after);
    });
}
