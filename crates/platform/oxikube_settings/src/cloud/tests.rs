use std::time::Duration;

use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<CloudSettings>();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn the_defaults_load_and_every_field_is_readable_and_default_json_says_so() {
    let store = store("{}");
    let settings = store.get::<CloudSettings>(None);
    assert_eq!(*settings, CloudSettings::default());
    assert!(settings.aws_enabled && settings.gcloud_enabled && settings.az_enabled);
    assert!(settings.aws_path.is_none() && settings.kubelogin_path.is_none());
    assert!(settings.aws_profiles.is_empty() && settings.az_subscriptions.is_empty());
    assert_eq!(settings.cache_ttl, Duration::from_secs(900));
    assert_eq!(settings.parallelism, 4);
    assert_eq!(settings.command_timeout, Duration::from_secs(30));
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());

    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).unwrap();
    let section = &defaults["cloud"];
    for key in [
        "providers",
        "aws_path",
        "gcloud_path",
        "az_path",
        "gke_gcloud_auth_plugin_path",
        "kubelogin_path",
        "aws",
        "gcloud",
        "az",
        "cache_ttl_secs",
        "parallelism",
        "command_timeout_secs",
    ] {
        assert!(!section[key].is_null(), "default.json lacks cloud.{key}");
    }
    assert_eq!(section["cache_ttl_secs"], 900);
    assert_eq!(section["parallelism"], 4);
}

#[test]
fn a_user_layer_sets_each_key() {
    let store = store(
        r#"{ "cloud": {
            "providers": { "gcloud": false },
            "aws_path": " /opt/aws ", "kubelogin_path": "/x/kubelogin",
            "aws": { "profiles": ["prod", " prod ", "", "dev"], "regions": ["eu-west-1"] },
            "gcloud": { "projects": ["p1"] },
            "az": { "subscriptions": ["Sub A"] },
            "cache_ttl_secs": 60, "parallelism": 8, "command_timeout_secs": 90 } }"#,
    );
    let s = store.get::<CloudSettings>(None);
    assert!(s.aws_enabled && !s.gcloud_enabled && s.az_enabled);
    assert!(s.is_enabled(CloudProvider::Aws) && !s.is_enabled(CloudProvider::Gcp));
    assert_eq!(
        s.aws_path.as_deref(),
        Some(std::path::Path::new("/opt/aws"))
    );
    assert_eq!(s.aws_profiles, ["prod", "dev"]);
    assert_eq!(s.aws_regions, ["eu-west-1"]);
    assert_eq!(s.gcloud_projects, ["p1"]);
    assert_eq!(s.az_subscriptions, ["Sub A"]);
    assert_eq!(s.cache_ttl, Duration::from_secs(60));
    assert_eq!(s.parallelism, 8);
    assert_eq!(s.command_timeout, Duration::from_secs(90));
}

#[test]
fn out_of_range_numbers_are_clamped() {
    let low = store(
        r#"{ "cloud": { "cache_ttl_secs": 1, "parallelism": 0, "command_timeout_secs": 0 } }"#,
    );
    let s = low.get::<CloudSettings>(None);
    assert_eq!(s.cache_ttl, Duration::from_secs(MIN_TTL_SECS));
    assert_eq!(s.parallelism, 1);
    assert_eq!(s.command_timeout, Duration::from_secs(MIN_TIMEOUT_SECS));
    let high = store(
        r#"{ "cloud": { "cache_ttl_secs": 99999999, "parallelism": 500, "command_timeout_secs": 99999 } }"#,
    );
    let s = high.get::<CloudSettings>(None);
    assert_eq!(s.cache_ttl, Duration::from_secs(MAX_TTL_SECS));
    assert_eq!(s.parallelism, 16);
    assert_eq!(s.command_timeout, Duration::from_secs(MAX_TIMEOUT_SECS));
}

#[test]
fn an_unknown_provider_is_reported_and_the_previous_value_stays() {
    let mut store = store(r#"{ "cloud": { "providers": { "az": false } } }"#);
    store
        .set_user_settings(
            r#"{ "cloud": { "providers": { "aws": false, "digitalocean": true } } }"#,
        )
        .unwrap();
    assert!(!store.diagnostics().is_empty());
    let s = store.get::<CloudSettings>(None);
    assert!(s.aws_enabled && !s.az_enabled, "{s:?}");
}

#[test]
fn a_wrong_type_is_reported() {
    let store = store(r#"{ "cloud": { "parallelism": "many" } }"#);
    assert!(!store.diagnostics().is_empty());
}

#[test]
fn the_schema_describes_every_key() {
    let schema = serde_json::to_string(&store("{}").json_schema()).unwrap();
    for key in [
        "\"cloud\"",
        "providers",
        "aws_path",
        "gcloud_path",
        "az_path",
        "gke_gcloud_auth_plugin_path",
        "kubelogin_path",
        "profiles",
        "regions",
        "projects",
        "subscriptions",
        "cache_ttl_secs",
        "parallelism",
        "command_timeout_secs",
        "EMPTY",
    ] {
        let key = key.to_lowercase();
        assert!(
            schema.to_lowercase().contains(&key),
            "{key} missing from the schema"
        );
    }
    assert!(
        schema.contains("every profile"),
        "the list semantics are described"
    );
}

/// Editing one value through the comment-preserving writer keeps the comments around it.
#[test]
fn editing_a_value_keeps_the_surrounding_comments() {
    let text = "{\n  // my clouds\n  \"cloud\": {\n    // keep it small\n    \"parallelism\": 4, // four\n    \"cache_ttl_secs\": 900\n  }\n}\n";
    let new = crate::new_text_for_update::<CloudSettings>(text, None, |content| {
        content.parallelism = Some(2);
    })
    .unwrap();
    assert!(
        new.contains("// my clouds") && new.contains("// keep it small"),
        "{new}"
    );
    assert!(new.contains("// four"), "{new}");
    assert!(new.contains("\"parallelism\": 2"), "{new}");
}
