//! The `log` setting against the embedded defaults and a live subscriber.

use super::*;
use crate::{LogConfig, build};
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_settings::SettingsStore;
use std::fs;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn default_json_spells_out_the_shipped_filter() {
    let defaults = oxikube_settings::jsonc::parse_jsonc_object(oxikube_assets::default_settings())
        .expect("default.json parses");
    assert_eq!(
        defaults["log"]["filter"].as_str(),
        Some(DEFAULT_DIRECTIVES),
        "default.json and DEFAULT_DIRECTIVES must stay equal"
    );
}

#[gpui::test]
fn default_json_carries_the_shipped_filter(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(store("{}"));
        assert_eq!(LogSettings::get_global(cx).filter, DEFAULT_DIRECTIVES);
    });
}

#[gpui::test]
fn the_user_file_overrides_the_filter(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(store(r#"{ "log": { "filter": "warn" } }"#));
        assert_eq!(LogSettings::get_global(cx).filter, "warn");
    });
}

fn config(dir: &std::path::Path) -> LogConfig {
    let mut config = LogConfig::new(dir);
    config.honour_rust_log = false;
    config
}

fn log_text(dir: &std::path::Path) -> String {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect()
}

#[gpui::test]
fn editing_the_setting_changes_the_running_filter(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (dispatch, guard) = build(&config(dir.path())).unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);
    let handle = guard.handle();
    cx.update(|cx| {
        cx.set_global(store("{}"));
        // Observers live as long as the subscription: keep it for the whole test.
        follow(cx, handle.clone()).detach();
    });
    tracing::debug!("debug-before");
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "log": { "filter": "debug" } }"#)
                .unwrap();
        });
    });
    tracing::debug!("debug-after");
    assert_eq!(handle.directives(), "debug");
    drop(guard);
    let text = log_text(dir.path());
    assert!(!text.contains("debug-before"), "{text}");
    assert!(text.contains("debug-after"), "{text}");
}

#[gpui::test]
fn an_invalid_filter_keeps_the_previous_one(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (_dispatch, guard) = build(&config(dir.path())).unwrap();
    let handle = guard.handle();
    cx.update(|cx| {
        cx.set_global(store(r#"{ "log": { "filter": "info=notalevel[" } }"#));
        let _subscription = follow(cx, handle.clone());
    });
    assert_eq!(handle.directives(), DEFAULT_DIRECTIVES);
}
