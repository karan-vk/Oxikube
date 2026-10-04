//! Merge precedence, error handling, change tracking and schema generation.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use serde_json::{Value, json};

use super::*;
use crate::test_support::{DEFAULTS, GeneralSettings, TerminalSettings, store};

fn cluster(n: u8) -> ClusterId {
    format!("{n:016x}").parse().unwrap()
}

fn at(id: &ClusterId) -> Option<SettingsLocation<'_>> {
    Some(SettingsLocation { cluster: id })
}

#[test]
fn defaults_apply_when_the_user_file_is_empty() {
    let mut store = store();
    store.set_user_settings("").unwrap();
    let terminal = store.get::<TerminalSettings>(None);
    assert_eq!(terminal.font_size, 12.0);
    assert_eq!(terminal.shell, "/bin/sh");
    assert_eq!(
        store.get::<GeneralSettings>(None),
        &GeneralSettings {
            ui_scale: 1.0,
            read_only: false
        }
    );
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn precedence_is_default_then_user_then_cluster() {
    let (a, b) = (cluster(1), cluster(2));
    let mut store = store();
    store
        .set_user_settings(&format!(
            r#"{{
                // user layer
                "read_only": false,
                "terminal": {{ "font_size": 14 }},
                "clusters": {{
                    "{a}": {{ "terminal": {{ "font_size": 16 }}, "read_only": true }},
                }},
            }}"#
        ))
        .unwrap();

    // User overrides one field; the sibling field still comes from default.json.
    let global = store.get::<TerminalSettings>(None);
    assert_eq!((global.font_size, global.shell.as_str()), (14.0, "/bin/sh"));
    // The cluster layer wins over both, again field by field.
    let in_a = store.get::<TerminalSettings>(at(&a));
    assert_eq!((in_a.font_size, in_a.shell.as_str()), (16.0, "/bin/sh"));
    assert!(store.get::<GeneralSettings>(at(&a)).read_only);
    // A cluster without overrides reads the global value.
    assert_eq!(store.get::<TerminalSettings>(at(&b)).font_size, 14.0);
    assert!(!store.get::<GeneralSettings>(at(&b)).read_only);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn null_and_removed_keys_fall_back_to_the_layer_below() {
    let mut store = store();
    store
        .set_user_settings(r#"{"terminal": {"font_size": 20}}"#)
        .unwrap();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 20.0);
    store
        .set_user_settings(r#"{"terminal": {"font_size": null}}"#)
        .unwrap();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 12.0);
    store
        .set_user_settings(r#"{"terminal": {"font_size": 20}}"#)
        .unwrap();
    store.set_user_settings("{}").unwrap();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 12.0);
}

#[test]
fn arrays_and_scalars_replace_maps_merge() {
    let mut store = store();
    store
        .set_user_settings(r#"{"terminal": {"args": ["-l"], "env": {"A": "1"}}}"#)
        .unwrap();
    let terminal = store.get::<TerminalSettings>(None);
    assert_eq!(terminal.args, vec!["-l".to_owned()]);
    assert_eq!(terminal.env.get("A").map(String::as_str), Some("1"));
}

#[test]
fn type_errors_keep_the_last_good_value() {
    let mut store = store();
    store
        .set_user_settings(r#"{"terminal": {"font_size": 14}, "ui_scale": 2}"#)
        .unwrap();
    let generation = store.generation::<TerminalSettings>();

    store
        .set_user_settings(r#"{"terminal": {"font_size": "big"}, "ui_scale": 3}"#)
        .unwrap();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 14.0);
    assert_eq!(store.generation::<TerminalSettings>(), generation);
    // The other setting in the same file still applies.
    assert_eq!(store.get::<GeneralSettings>(None).ui_scale, 3.0);
    match store.diagnostics() {
        [
            SettingsDiagnostic::InvalidValue {
                cluster: None,
                message,
                ..
            },
        ] => assert!(message.starts_with("terminal.font_size:"), "{message}"),
        other => panic!("unexpected diagnostics {other:?}"),
    }
}

#[test]
fn a_type_error_with_no_last_good_value_uses_the_defaults() {
    let mut store = SettingsStore::without_registered(DEFAULTS).unwrap();
    store
        .set_user_settings(r#"{"terminal": {"font_size": [1]}}"#)
        .unwrap();
    store.register_setting::<TerminalSettings>();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 12.0);
    assert!(matches!(
        store.diagnostics(),
        [SettingsDiagnostic::InvalidValue { .. }]
    ));
}

#[test]
fn a_type_error_in_a_cluster_keeps_that_clusters_last_good_value() {
    let a = cluster(1);
    let mut store = store();
    store
        .set_user_settings(&format!(
            r#"{{"clusters": {{"{a}": {{"terminal": {{"font_size": 18}}}}}}}}"#
        ))
        .unwrap();
    store
        .set_user_settings(&format!(
            r#"{{"clusters": {{"{a}": {{"terminal": {{"font_size": false}}}}}}}}"#
        ))
        .unwrap();
    assert_eq!(store.get::<TerminalSettings>(at(&a)).font_size, 18.0);
    assert!(matches!(
        store.diagnostics(),
        [SettingsDiagnostic::InvalidValue { cluster: Some(id), .. }] if id == a.as_str()
    ));
}

#[test]
fn invalid_json_keeps_the_last_good_layer_and_reports_it() {
    let mut store = store();
    store
        .set_user_settings(r#"{"terminal": {"font_size": 15}}"#)
        .unwrap();
    let err = store
        .set_user_settings(r#"{"terminal": {"font_size": 16"#)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 15.0);
    assert!(matches!(
        store.diagnostics(),
        [SettingsDiagnostic::InvalidJson { .. }]
    ));
    assert_eq!(
        store.raw_user_settings().get("terminal"),
        Some(&json!({"font_size": 15}))
    );
}

#[test]
fn unknown_keys_are_reported_but_do_not_block_loading() {
    let a = cluster(1);
    let mut store = store();
    store
        .set_user_settings(&format!(
            r#"{{
                "$schema": "./settings.schema.json",
                "terminl": {{}},
                "terminal": {{ "font_size": 13, "fnot": "x", "env": {{ "ANY": "ok" }} }},
                "clusters": {{ "{a}": {{ "colour": "red" }} }},
            }}"#
        ))
        .unwrap();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 13.0);
    let mut unknown: Vec<_> = store
        .diagnostics()
        .iter()
        .map(|d| match d {
            SettingsDiagnostic::UnknownKey { path } => path.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    unknown.sort();
    assert_eq!(
        unknown,
        vec![
            format!("clusters.{a}.colour"),
            "terminal.fnot".to_owned(),
            "terminl".to_owned(),
        ]
    );
}

#[test]
fn generations_move_only_for_settings_whose_value_changed() {
    let mut store = store();
    store.set_user_settings("{}").unwrap();
    let (terminal, general) = (
        store.generation::<TerminalSettings>(),
        store.generation::<GeneralSettings>(),
    );

    store.set_user_settings(r#"{"ui_scale": 2}"#).unwrap();
    assert_eq!(store.generation::<TerminalSettings>(), terminal);
    assert_eq!(store.generation::<GeneralSettings>(), general + 1);

    // A reformatted file with the same values changes nothing.
    store
        .set_user_settings("// comment\n{ \"ui_scale\": 2.0, }")
        .unwrap();
    assert_eq!(store.generation::<GeneralSettings>(), general + 1);

    // Adding a cluster override counts as a change of that setting only.
    store
        .set_user_settings(&format!(
            r#"{{"ui_scale": 2, "clusters": {{"{}": {{"terminal": {{"shell": "zsh"}}}}}}}}"#,
            cluster(3)
        ))
        .unwrap();
    assert_eq!(store.generation::<TerminalSettings>(), terminal + 1);
    assert_eq!(store.generation::<GeneralSettings>(), general + 1);
}

#[test]
fn registering_after_load_resolves_from_the_current_layers() {
    let mut store = SettingsStore::without_registered(DEFAULTS).unwrap();
    store
        .set_user_settings(r#"{"terminal": {"font_size": 21}}"#)
        .unwrap();
    // Nothing registered yet: every key is unknown.
    assert!(!store.diagnostics().is_empty());
    assert!(store.try_get::<TerminalSettings>(None).is_none());

    store.register_setting::<TerminalSettings>();
    store.register_setting::<TerminalSettings>();
    assert_eq!(store.get::<TerminalSettings>(None).font_size, 21.0);
    store.register_setting::<GeneralSettings>();
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn override_global_replaces_the_value_until_the_next_reload() {
    let mut store = store();
    let generation = store.generation::<GeneralSettings>();
    store.override_global(GeneralSettings {
        ui_scale: 9.0,
        read_only: true,
    });
    assert_eq!(store.get::<GeneralSettings>(None).ui_scale, 9.0);
    assert_eq!(store.generation::<GeneralSettings>(), generation + 1);
    store.set_user_settings(r#"{"ui_scale": 1.5}"#).unwrap();
    assert_eq!(store.get::<GeneralSettings>(None).ui_scale, 1.5);
}

#[test]
#[should_panic(expected = "is not registered")]
fn reading_an_unregistered_setting_panics() {
    SettingsStore::empty().get::<TerminalSettings>(None);
}

/// A setting registered through the macro, i.e. through `inventory`.
#[derive(PartialEq)]
struct InventorySettings(bool);

impl Settings for InventorySettings {
    const KEY: Option<&'static str> = Some("inventory_probe");
    type Content = Option<bool>;

    fn from_content(content: Option<bool>) -> Self {
        Self(content.unwrap_or_default())
    }
}

crate::register_settings!(InventorySettings);

#[test]
fn new_registers_every_inventory_setting() {
    let mut store = SettingsStore::new(r#"{"inventory_probe": true}"#).unwrap();
    assert!(store.get::<InventorySettings>(None).0);
    store
        .set_user_settings(r#"{"inventory_probe": false}"#)
        .unwrap();
    assert!(!store.get::<InventorySettings>(None).0);
}

#[test]
fn the_embedded_defaults_load_cleanly() {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
    store
        .set_user_settings(oxikube_assets::initial_user_settings_content())
        .unwrap();
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn schema_is_deterministic_and_covers_every_layer() {
    let schema = store().json_schema();
    let mut reversed = SettingsStore::without_registered(DEFAULTS).unwrap();
    reversed.register_setting::<GeneralSettings>();
    reversed.register_setting::<TerminalSettings>();
    assert_eq!(
        crate::schema::to_schema_text(&schema),
        crate::schema::to_schema_text(&reversed.json_schema()),
        "registration order must not change the schema"
    );

    let properties = &schema["properties"];
    // Keyed content at its key, root content flattened, plus the reserved keys.
    assert!(properties["terminal"]["$ref"].is_string());
    assert!(properties["ui_scale"].is_object());
    assert!(properties["read_only"].is_object());
    assert_eq!(
        properties["clusters"]["additionalProperties"]["$ref"],
        "#/$defs/ClusterSettings"
    );
    assert_eq!(schema["additionalProperties"], Value::Bool(false));
    assert_eq!(schema["allowTrailingCommas"], Value::Bool(true));
    let cluster_props = &schema["$defs"]["ClusterSettings"]["properties"];
    assert!(cluster_props["terminal"].is_object() && cluster_props["ui_scale"].is_object());
    assert_eq!(
        schema["$defs"]["TerminalContent"]["additionalProperties"],
        Value::Bool(false)
    );
    // Keys are sorted at every level.
    let keys: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}
