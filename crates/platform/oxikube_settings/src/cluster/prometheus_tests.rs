//! The `prometheus` block (E13-S05): defaults, field-by-field layering, validation, the schema,
//! and that no secret ever lands in the file.

use oxikube_domain::MetricsProviderKind;
use oxikube_ports::{ClusterPrefs, MetricsMode, PrometheusAuthKind};

use super::{ClusterSettings, PrometheusContent};
use crate::diagnostics::SettingsDiagnostic;
use crate::settings::SettingsLocation;
use crate::store::SettingsStore;
use crate::update::new_text_for_update;

const PROD: &str = "3f2a9c1b7d4e8a60";
const LAB: &str = "0011223344556677";

fn id(text: &str) -> oxikube_domain::ids::ClusterId {
    text.parse().unwrap()
}

fn load(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<ClusterSettings>();
    store.set_user_settings(user).unwrap();
    store
}

fn prefs<'a>(store: &'a SettingsStore, cluster: &str) -> &'a ClusterPrefs {
    let cluster = id(cluster);
    store
        .get::<ClusterSettings>(Some(SettingsLocation { cluster: &cluster }))
        .prefs()
}

fn prometheus(store: &SettingsStore, cluster: &str) -> oxikube_ports::PrometheusOverride {
    prefs(store, cluster).prometheus.clone().unwrap_or_default()
}

#[test]
fn the_shipped_defaults_are_on_and_auto_with_nothing_configured() {
    let store = load("{}");
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
    let value = prometheus(&store, PROD);
    assert!(value.enabled);
    assert_eq!(value.effective_mode(), MetricsMode::Auto);
    assert_eq!(value.url, None);
    assert_eq!(value.auth, PrometheusAuthKind::None);
    // Defaults carry no override: a cluster without a block reads the same.
    assert_eq!(prefs(&store, PROD).prometheus, None);
}

#[test]
fn a_url_or_a_path_makes_the_block_manual_unless_the_provider_says_otherwise() {
    let store = load(&format!(
        r#"{{ "clusters": {{
          "{PROD}": {{ "prometheus": {{ "url": "https://vm.example.com" }} }},
          "{LAB}": {{ "prometheus": {{ "path": "monitoring/vmsingle:8429", "provider": "auto" }} }}
        }} }}"#
    ));
    assert_eq!(prometheus(&store, PROD).mode, MetricsMode::Manual);
    assert_eq!(prometheus(&store, LAB).mode, MetricsMode::Auto);
}

#[test]
fn enabled_false_switches_metrics_off_for_one_cluster_only() {
    let store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "prometheus": {{ "enabled": false }} }} }} }}"#
    ));
    assert_eq!(prometheus(&store, PROD).effective_mode(), MetricsMode::Off);
    assert_eq!(prometheus(&store, LAB).effective_mode(), MetricsMode::Auto);
}

#[test]
fn the_top_level_block_is_a_default_the_cluster_overrides_field_by_field() {
    let store = load(&format!(
        r#"{{
          "prometheus": {{ "provider": "manual", "url": "https://prom.example.com",
                           "path_prefix": "/prometheus", "auth_secret": "shared" }},
          "clusters": {{ "{PROD}": {{ "prometheus": {{
            "url": "https://other.example.com", "auth": "basic", "auth_user": "ops" }} }} }}
        }}"#
    ));
    let lab = prometheus(&store, LAB);
    assert_eq!(lab.url.as_deref(), Some("https://prom.example.com"));
    assert_eq!(
        lab.auth,
        PrometheusAuthKind::Bearer,
        "a secret alone is a bearer token"
    );
    let prod = prometheus(&store, PROD);
    assert_eq!(prod.url.as_deref(), Some("https://other.example.com"));
    assert_eq!(
        prod.path_prefix.as_deref(),
        Some("/prometheus"),
        "inherited"
    );
    assert_eq!(prod.auth, PrometheusAuthKind::Basic);
    assert_eq!(prod.auth_user.as_deref(), Some("ops"));
    assert_eq!(prod.auth_secret.unwrap().to_string(), "prometheus/shared");
}

#[test]
fn a_secret_and_a_user_without_an_auth_kind_mean_basic() {
    let store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "prometheus": {{
          "url": "https://p", "auth_user": "ops", "auth_secret": "pw" }} }} }} }}"#
    ));
    assert_eq!(prometheus(&store, PROD).auth, PrometheusAuthKind::Basic);
}

#[test]
fn bad_values_are_refused_with_the_field_path_and_without_echoing_secrets() {
    for (bad, needle) in [
        (r#"{"provider": "sometimes"}"#, "prometheus.provider"),
        (r#"{"kind": "grafana"}"#, "prometheus.kind"),
        (r#"{"path": "prometheus"}"#, "prometheus.path"),
        (r#"{"path_prefix": "/a/../b"}"#, "prometheus.path_prefix"),
        (
            r#"{"path_prefix": "/a?token=hunter2"}"#,
            "prometheus.path_prefix",
        ),
        (r#"{"auth": "oauth"}"#, "prometheus.auth"),
        (r#"{"enabled": "yes"}"#, "prometheus.enabled"),
    ] {
        let store = load(&format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "prometheus": {bad} }} }} }}"#
        ));
        let [SettingsDiagnostic::InvalidValue { message, .. }] = store.diagnostics() else {
            panic!("{bad}: {:?}", store.diagnostics());
        };
        assert!(message.contains(needle), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
    }
}

#[test]
fn the_token_itself_has_no_place_in_the_file() {
    let store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "prometheus": {{ "token": "hunter2" }} }} }} }}"#
    ));
    assert!(
        !store.diagnostics().is_empty(),
        "an unknown key such as `token` must be reported, not accepted"
    );
    // The content type cannot hold a secret: its only credential field is a keychain name.
    let text = serde_json::to_string(&PrometheusContent::default()).unwrap();
    assert_eq!(text, "{}");
}

#[test]
fn edits_through_the_comment_preserving_editor_keep_the_block() {
    let original = r#"{
  // my clusters
  "clusters": { "3f2a9c1b7d4e8a60": { "prometheus": { "url": "https://prom.example.com" } } }
}"#;
    let edited = new_text_for_update::<ClusterSettings>(original, Some(&id(PROD)), |content| {
        content.prometheus.as_mut().unwrap().enabled = Some(false);
    })
    .unwrap();
    assert!(edited.contains("// my clusters"), "{edited}");
    let store = load(&edited);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
    let value = prometheus(&store, PROD);
    assert_eq!(value.effective_mode(), MetricsMode::Off);
    assert_eq!(value.url.as_deref(), Some("https://prom.example.com"));
}

#[test]
fn kinds_round_trip_through_the_settings_ids() {
    for kind in MetricsProviderKind::ALL {
        let store = load(&format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "prometheus": {{ "kind": "{}" }} }} }} }}"#,
            kind.id()
        ));
        assert_eq!(prometheus(&store, PROD).kind, Some(kind));
    }
}

#[test]
fn the_schema_lists_the_keys_the_modes_and_the_kinds() {
    let schema = load("{}").json_schema();
    let content = &schema["$defs"]["PrometheusContent"]["properties"];
    for key in [
        "enabled",
        "provider",
        "kind",
        "url",
        "path",
        "path_prefix",
        "auth",
        "auth_user",
        "auth_secret",
    ] {
        assert!(
            content[key]["description"].is_string(),
            "{key} is undocumented"
        );
    }
    let modes = schema["$defs"]["ProviderModeContent"]["oneOf"]
        .as_array()
        .unwrap();
    let names: Vec<_> = modes.iter().map(|m| m["const"].as_str().unwrap()).collect();
    assert_eq!(names, ["auto", "manual", "off"]);
    let kinds = content["kind"]["enum"].as_array().unwrap();
    assert_eq!(
        kinds.len(),
        MetricsProviderKind::ALL.len() + 1,
        "all kinds and null"
    );
}
