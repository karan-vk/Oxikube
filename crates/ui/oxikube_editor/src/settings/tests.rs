//! The `editor` settings through the real store: defaults, the layers, invalid values, and that
//! `default.json` and the schema carry every key.

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_settings::{SettingsLocation, SettingsStore};
use oxikube_ui::editor::{DEFAULT_TAB_SIZE, EditorFont};
use schemars::schema_for;
use serde_json::{Value, json};

use super::*;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    store.set_user_settings(user).expect("valid settings");
    store
}

fn global(user: &str) -> EditorSettings {
    store(user).get::<EditorSettings>(None).clone()
}

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

#[test]
fn default_json_spells_out_every_default() {
    let settings = global("{}");
    assert_eq!(
        settings,
        EditorSettings {
            tab_size: 2,
            font_family: None,
            font_size: None,
            soft_wrap: false,
            show_managed_fields: false,
            mask_secrets: true,
            validation: true,
            field_manager: "oxikube".into(),
            problems: Vec::new(),
        },
        "assets/settings/default.json and the code's defaults disagree"
    );
    assert_eq!(settings, EditorSettings::default());
}

/// The keys of `EditorSettingsContent`, from its schema: a field added to the struct is a key
/// every check below must find.
fn content_keys() -> Vec<String> {
    let schema = schema_for!(EditorSettingsContent).to_value();
    let mut keys: Vec<String> = schema["properties"]
        .as_object()
        .expect("an object schema")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

#[test]
fn every_setting_has_a_default_and_a_schema_entry() {
    let keys = content_keys();
    assert_eq!(keys.len(), 8, "the story's eight settings: {keys:?}");

    let defaults = oxikube_settings::jsonc::parse_jsonc_object(oxikube_assets::default_settings())
        .expect("default.json parses");
    let block = defaults["editor"].as_object().expect("an editor block");
    let schema: Value = serde_json::from_str(oxikube_assets::settings_schema()).expect("schema");
    let described = &schema["properties"]["editor"];
    // The generator `$ref`s a struct into `$defs`.
    let properties = match described["$ref"].as_str() {
        Some(reference) => {
            let name = reference.rsplit('/').next().expect("a ref");
            &schema["$defs"][name]["properties"]
        }
        None => &described["properties"],
    };
    for key in &keys {
        assert!(block.contains_key(key), "default.json has no editor.{key}");
        let entry = &properties[key];
        assert!(
            entry.is_object(),
            "settings.schema.json has no editor.{key}"
        );
        assert!(
            entry["description"].as_str().is_some_and(|d| !d.is_empty()),
            "editor.{key} has no schema description"
        );
    }
    assert_eq!(
        block.len(),
        keys.len(),
        "default.json has an editor key the content does not know"
    );
}

#[test]
fn the_secrets_description_says_nothing_decoded_is_persisted() {
    let schema = schema_for!(EditorSettingsContent).to_value();
    let description = schema["properties"]["mask_secrets"]["description"]
        .as_str()
        .expect("a description");
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(description.contains("never writes decoded Secret data to disk"));
}

#[test]
fn a_user_value_overrides_the_default_and_keeps_the_rest() {
    let settings = global(
        r#"{ "editor": { "tab_size": 4, "soft_wrap": true, "font_family": "JetBrains Mono",
             "font_size": 14, "validation": false, "mask_secrets": false,
             "show_managed_fields": true, "field_manager": "my-tool" } }"#,
    );
    assert_eq!(settings.tab_size, 4);
    assert!(settings.soft_wrap);
    assert_eq!(settings.font_family.as_deref(), Some("JetBrains Mono"));
    assert_eq!(settings.font_size, Some(14.0));
    assert!(!settings.validation);
    assert!(!settings.mask_secrets);
    assert!(settings.show_managed_fields);
    assert_eq!(settings.field_manager, "my-tool");
    assert!(settings.problems.is_empty());

    let partial = global(r#"{ "editor": { "tab_size": 8 } }"#);
    assert_eq!(partial.tab_size, 8);
    assert!(partial.mask_secrets, "untouched keys keep their default");
    assert_eq!(partial.field_manager, "oxikube");
}

#[test]
fn a_cluster_override_resolves_ahead_of_the_user_value() {
    let prod = cluster("prod");
    let other = cluster("dev");
    let user = format!(
        r#"{{ "editor": {{ "field_manager": "me", "tab_size": 4 }},
             "clusters": {{ "{}": {{ "editor": {{ "field_manager": "argocd-client" }} }} }} }}"#,
        prod.as_str()
    );
    let store = store(&user);
    let at = |cluster| {
        store
            .get::<EditorSettings>(Some(SettingsLocation { cluster }))
            .clone()
    };
    let on_prod = at(&prod);
    assert_eq!(on_prod.field_manager, "argocd-client");
    assert_eq!(
        on_prod.tab_size, 4,
        "the cluster block merges over the user's"
    );
    let elsewhere = at(&other);
    assert_eq!(elsewhere.field_manager, "me");
}

#[test]
fn an_invalid_field_manager_falls_back_to_oxikube_and_says_so() {
    let too_long = "x".repeat(MAX_FIELD_MANAGER_LEN + 1);
    for bad in ["", " ", "has space", "tab\there", too_long.as_str()] {
        let user = json!({ "editor": { "field_manager": bad, "tab_size": 6 } }).to_string();
        let settings = global(&user);
        assert_eq!(settings.field_manager, DEFAULT_FIELD_MANAGER, "{bad:?}");
        assert_eq!(settings.tab_size, 6, "the rest of the block still applies");
        assert_eq!(settings.problems.len(), 1, "{bad:?}");
        assert!(settings.problems[0].starts_with("editor.field_manager"));
        assert!(
            bad.trim().is_empty() || !settings.problems[0].contains(bad),
            "a value from the file is not echoed"
        );
    }
    let longest = "y".repeat(MAX_FIELD_MANAGER_LEN);
    let ok = global(&json!({ "editor": { "field_manager": longest } }).to_string());
    assert_eq!(ok.field_manager.len(), MAX_FIELD_MANAGER_LEN);
    assert!(ok.problems.is_empty());
}

#[test]
fn a_non_string_field_manager_is_rejected_with_a_diagnostic_and_the_default_is_used() {
    let mut store =
        store(r#"{ "editor": { "field_manager": 42, "tab_size": 3, "soft_wrap": true } }"#);
    let settings = store.get::<EditorSettings>(None).clone();
    assert_eq!(settings.field_manager, DEFAULT_FIELD_MANAGER);
    // The other keys of the block are not lost to the one typo.
    assert_eq!(settings.tab_size, 3);
    assert!(settings.soft_wrap);
    assert_eq!(
        settings.problems,
        ["editor.field_manager has the wrong type: using the default"]
    );
    // The store reports it too, naming the field.
    let diagnostics: Vec<String> = store
        .diagnostics()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("editor.field_manager")),
        "{diagnostics:?}"
    );
    // A later good edit recovers.
    store
        .set_user_settings(r#"{ "editor": { "field_manager": "fixed" } }"#)
        .unwrap();
    assert_eq!(store.get::<EditorSettings>(None).field_manager, "fixed");
}

#[test]
fn a_bad_value_after_a_good_one_does_not_keep_the_good_one() {
    let mut store = store(r#"{ "editor": { "field_manager": "mine" } }"#);
    store
        .set_user_settings(r#"{ "editor": { "field_manager": ["a"] } }"#)
        .unwrap();
    assert_eq!(
        store.get::<EditorSettings>(None).field_manager,
        DEFAULT_FIELD_MANAGER,
        "the default, not the last good value"
    );
}

#[test]
fn a_negative_tab_size_is_rejected_and_the_default_is_used() {
    for bad in [-1, 0, 17, 1000] {
        let settings = global(&json!({ "editor": { "tab_size": bad } }).to_string());
        assert_eq!(settings.tab_size, DEFAULT_TAB_SIZE, "{bad}");
        assert_eq!(settings.problems.len(), 1, "{bad}");
        assert!(settings.problems[0].starts_with("editor.tab_size"));
    }
    for good in [1, 2, 16] {
        let settings = global(&json!({ "editor": { "tab_size": good } }).to_string());
        assert_eq!(settings.tab_size, good as usize);
        assert!(settings.problems.is_empty());
    }
    let wrong_type = global(r#"{ "editor": { "tab_size": "two" } }"#);
    assert_eq!(wrong_type.tab_size, DEFAULT_TAB_SIZE);
    assert_eq!(
        wrong_type.problems,
        ["editor.tab_size has the wrong type: using the default"]
    );
}

#[test]
fn the_font_is_validated_and_blank_means_the_theme() {
    let settings = global(r#"{ "editor": { "font_family": "  ", "font_size": 5 } }"#);
    assert_eq!(settings.font_family, None);
    assert_eq!(settings.font_size, None);
    assert_eq!(settings.problems.len(), 1, "{:?}", settings.problems);
    assert_eq!(settings.font(), EditorFont::default());

    let big = global(r#"{ "editor": { "font_size": 80 } }"#);
    assert_eq!(big.font_size, None);
    let fine = global(r#"{ "editor": { "font_family": " Menlo ", "font_size": 12.5 } }"#);
    assert_eq!(
        fine.font(),
        EditorFont {
            family: Some("Menlo".into()),
            size: Some(12.5)
        }
    );
}

#[test]
fn only_a_real_change_bumps_the_generation() {
    let mut store = store("{}");
    let before = store.generation::<EditorSettings>();
    store
        .set_user_settings(r#"{ "theme": { "mode": "dark" } }"#)
        .unwrap();
    assert_eq!(store.generation::<EditorSettings>(), before);
    store
        .set_user_settings(r#"{ "editor": { "tab_size": 4 } }"#)
        .unwrap();
    assert_eq!(store.generation::<EditorSettings>(), before + 1);
}

#[test]
fn the_field_manager_rule() {
    assert!(valid_field_manager("oxikube"));
    assert!(valid_field_manager("kubectl-client-side-apply"));
    assert!(!valid_field_manager(""));
    assert!(!valid_field_manager("a b"));
    assert!(
        !valid_field_manager("a\u{a0}b"),
        "a non-breaking space is whitespace"
    );
}
