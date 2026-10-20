use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<PortForwardSettings>();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn the_defaults_are_loopback_strict_and_os_assigned_and_default_json_says_so() {
    let store = store("{}");
    let settings = store.get::<PortForwardSettings>(None);
    assert_eq!(*settings, PortForwardSettings::default());
    assert_eq!(settings.default_bind.to_string(), "127.0.0.1");
    assert!(!settings.allowed_in_readonly && !settings.annotations);
    assert!(settings.port_range.is_none());
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());

    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).unwrap();
    let section = &defaults["portforward"];
    assert_eq!(section["default_bind"], "127.0.0.1");
    assert_eq!(section["allowed_in_readonly"], false);
    assert_eq!(section["annotations"], false);
}

#[test]
fn a_user_layer_sets_each_key() {
    let store = store(
        r#"{ "portforward": { "default_bind": "0.0.0.0", "allowed_in_readonly": true,
             "port_range": [20000, 40000], "annotations": true } }"#,
    );
    let s = store.get::<PortForwardSettings>(None);
    assert_eq!(s.default_bind.to_string(), "0.0.0.0");
    assert!(s.allowed_in_readonly && s.annotations);
    assert_eq!(s.port_range, PortRange::new(20000, 40000));
}

#[test]
fn a_bind_that_is_not_an_ip_is_reported_and_the_previous_value_stays() {
    let mut store = store(r#"{ "portforward": { "default_bind": "::1" } }"#);
    store
        .set_user_settings(r#"{ "portforward": { "default_bind": "localhost" } }"#)
        .unwrap();
    assert!(!store.diagnostics().is_empty());
    assert_eq!(
        store
            .get::<PortForwardSettings>(None)
            .default_bind
            .to_string(),
        "::1"
    );
}

#[test]
fn an_invalid_range_is_ignored() {
    for bad in ["[80, 90]", "[5000, 4000]"] {
        let store = store(&format!(
            r#"{{ "portforward": {{ "port_range": {bad} }} }}"#
        ));
        assert!(store.get::<PortForwardSettings>(None).port_range.is_none());
    }
    // A wrong shape is a type error, reported.
    let store = store(r#"{ "portforward": { "port_range": [1, 2, 3] } }"#);
    assert!(!store.diagnostics().is_empty());
}

#[test]
fn a_cluster_overrides_single_keys_and_inherits_the_rest() {
    let id = "3f2a9c1b7d4e8a60";
    let store = store(&format!(
        r#"{{ "portforward": {{ "default_bind": "10.0.0.1", "port_range": [30000, 31000] }},
            "clusters": {{ "{id}": {{ "portforward": {{ "allowed_in_readonly": true }} }} }} }}"#
    ));
    let cluster: ClusterId = id.parse().unwrap();
    let global = store.get::<PortForwardSettings>(None);
    assert!(!global.allowed_in_readonly);
    let located =
        store.get::<PortForwardSettings>(Some(crate::SettingsLocation { cluster: &cluster }));
    assert!(located.allowed_in_readonly);
    assert_eq!(located.default_bind.to_string(), "10.0.0.1");
    assert_eq!(located.port_range, PortRange::new(30000, 31000));
}

#[test]
fn the_schema_describes_the_keys() {
    let schema = serde_json::to_string(&store("{}").json_schema()).unwrap();
    for key in [
        "portforward",
        "default_bind",
        "allowed_in_readonly",
        "port_range",
    ] {
        assert!(schema.contains(key), "{key} missing from {schema}");
    }
}

/// Audit E15 (#18): `annotations` was documented as annotating forwarded objects (a cluster
/// write), while it only turns on reading k9s pod annotations as dialog suggestions.
#[test]
fn the_annotations_key_is_described_as_reading_k9s_suggestions_not_writing() {
    let schema = store("{}").json_schema();
    let text = serde_json::to_string(&schema).unwrap();
    assert!(text.contains("k9scli.io/port-forwards"), "{text}");
    assert!(!text.contains("Annotate forwarded objects"), "{text}");
    let defaults = include_str!("../../../oxikube_assets/assets/settings/default.json");
    assert!(!defaults.contains("annotate the objects you forward"));
    assert!(defaults.contains("k9scli.io/port-forwards"));
}
