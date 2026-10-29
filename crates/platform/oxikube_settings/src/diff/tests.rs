use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<DiffSettings>();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn the_default_ignores_the_tooling_annotations_and_default_json_says_so() {
    let store = store("{}");
    let settings = store.get::<DiffSettings>(None);
    assert_eq!(*settings, DiffSettings::default());
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).unwrap();
    assert_eq!(
        defaults["diff"]["ignore_paths"],
        serde_json::json!(DiffSettings::default_ignore_paths())
    );
}

#[test]
fn a_user_list_replaces_the_default_and_a_reload_takes_effect_at_once() {
    let mut store = store(r#"{ "diff": { "ignore_paths": ["/spec/replicas"] } }"#);
    assert_eq!(
        store.get::<DiffSettings>(None).ignore_paths,
        ["/spec/replicas"]
    );
    store
        .set_user_settings(r#"{ "diff": { "ignore_paths": [] } }"#)
        .unwrap();
    assert!(store.get::<DiffSettings>(None).ignore_paths.is_empty());
    store.set_user_settings("{}").unwrap();
    assert_eq!(
        store.get::<DiffSettings>(None).ignore_paths,
        DiffSettings::default_ignore_paths()
    );
}

#[test]
fn entries_that_are_not_pointers_are_dropped() {
    let store =
        store(r#"{ "diff": { "ignore_paths": ["metadata.labels", "/ok/*", " /x ", "/"] } }"#);
    assert_eq!(
        store.get::<DiffSettings>(None).ignore_paths,
        ["/ok/*", "/x"]
    );
}

#[test]
fn a_wrong_type_is_reported_and_the_previous_value_stays() {
    let mut store = store(r#"{ "diff": { "ignore_paths": ["/a"] } }"#);
    store
        .set_user_settings(r#"{ "diff": { "ignore_paths": "/b" } }"#)
        .unwrap();
    assert!(!store.diagnostics().is_empty());
    assert_eq!(store.get::<DiffSettings>(None).ignore_paths, ["/a"]);
}
