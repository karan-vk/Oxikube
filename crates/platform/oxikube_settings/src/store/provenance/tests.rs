//! The layer badge logic: default-only, user override and cluster override values.

use oxikube_domain::ids::ClusterId;
use serde_json::json;

use super::SettingSource;
use crate::test_support::store;

fn cluster(n: u8) -> ClusterId {
    format!("{n:016x}").parse().expect("16 hex characters")
}

fn loaded(text: &str) -> crate::SettingsStore {
    let mut store = store();
    store.set_user_settings(text).expect("valid user settings");
    store
}

#[test]
fn a_field_only_in_the_defaults_comes_from_the_defaults() {
    let store = loaded("{}");
    assert_eq!(
        store.field_source("terminal.font_size", None),
        SettingSource::Default
    );
    assert_eq!(
        store.effective_value("terminal.font_size", None),
        Some(&json!(12))
    );
    assert_eq!(store.default_value("terminal.font_size"), Some(&json!(12)));
}

#[test]
fn a_field_the_user_sets_comes_from_the_user_file() {
    let store = loaded(r#"{ "terminal": { "font_size": 14 }, "read_only": true }"#);
    assert_eq!(
        store.field_source("terminal.font_size", None),
        SettingSource::User
    );
    assert_eq!(store.field_source("read_only", None), SettingSource::User);
    // The sibling stays a default; the section as a whole is touched by the user.
    assert_eq!(
        store.field_source("terminal.shell", None),
        SettingSource::Default
    );
    assert_eq!(store.field_source("terminal", None), SettingSource::User);
    assert_eq!(
        store.effective_value("terminal.font_size", None),
        Some(&json!(14))
    );
    assert_eq!(store.default_value("terminal.font_size"), Some(&json!(12)));
}

#[test]
fn a_cluster_override_wins_only_at_that_cluster() {
    let (a, b) = (cluster(1), cluster(2));
    let store = loaded(&format!(
        r#"{{
            "terminal": {{ "font_size": 14 }},
            "clusters": {{ "{a}": {{ "terminal": {{ "font_size": 16 }} }} }},
        }}"#
    ));
    assert_eq!(
        store.field_source("terminal.font_size", Some(&a)),
        SettingSource::Cluster
    );
    assert_eq!(
        store.effective_value("terminal.font_size", Some(&a)),
        Some(&json!(16))
    );
    // Another cluster, and the global value, still read the user layer.
    assert_eq!(
        store.field_source("terminal.font_size", Some(&b)),
        SettingSource::User
    );
    assert_eq!(
        store.effective_value("terminal.font_size", Some(&b)),
        Some(&json!(14))
    );
    assert_eq!(
        store.field_source("terminal.font_size", None),
        SettingSource::User
    );
    // A field the cluster block leaves alone falls through to the defaults.
    assert_eq!(
        store.field_source("terminal.shell", Some(&a)),
        SettingSource::Default
    );
}

#[test]
fn unknown_and_reserved_paths_are_defaults_without_a_value() {
    let store = loaded(r#"{ "clusters": {} }"#);
    assert_eq!(
        store.field_source("nope.nothing", None),
        SettingSource::Default
    );
    assert_eq!(store.field_source("clusters", None), SettingSource::Default);
    assert_eq!(store.field_source("", None), SettingSource::Default);
    assert_eq!(store.effective_value("nope", None), None);
    assert_eq!(SettingSource::Cluster.label(), "Cluster");
}
